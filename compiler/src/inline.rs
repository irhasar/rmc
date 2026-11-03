//! Function inlining, run on the AST after semantic analysis and before code
//! generation.
//!
//! Only small, straight-line, scalar functions are inlined. A getter like
//! `Wide::total` is a handful of `add`s around a reference, and the call it
//! replaces (argument setup, a jump, a return) costs more than the body.
//! Inlining it also exposes the arithmetic to the code generator's peephole
//! passes, which is where the real win comes from: the `refs` hot loop reads a
//! wide struct through three `&self` methods, and inlining turns three calls
//! per iteration into none.
//!
//! The pass is deliberately conservative. A candidate must:
//!
//! * have a scalar or unit return and scalar parameters (a reference is a
//!   scalar; aggregates are passed and returned by pointer and are left alone),
//!   and take no `self` by value;
//! * have a body with no loop, no nested call and no `return` except possibly
//!   as its last statement (so it is straight-line and inlining cannot add
//!   control flow or unbounded work);
//! * be small, at most [`MAX_NODES`] expression nodes.
//!
//! Expansion is a slot remap: each inlined call is given a fresh region of the
//! caller's frame, the callee's own slots are shifted into it, and a reference
//! to a parameter is replaced by the argument expression. The name-to-slot
//! binding is the one sema already computed, so no name lookup happens here.

use crate::ast::*;
use std::collections::HashSet;

/// Largest candidate body, in expression nodes: enough for an accessor or a
/// small arithmetic helper, small enough that inlining cannot bloat a caller.
/// A two-level helper counts high once its callees are inlined into it — the
/// `refs` `sum_via_ref` is 35 nodes after its two accessors land — so the
/// ceiling is set above that rather than at the accessor size.
const MAX_NODES: usize = 48;

/// Most call sites expanded across the whole program. Bounds compile time and
/// code size; a hot loop body is inlined once, not once per iteration, so this
/// only bites on a program with very many distinct call sites.
const BUDGET: usize = 8192;

/// Inline eligible calls in every function of `prog`.
///
/// Runs to a fixpoint. A function only becomes a candidate once it is itself
/// call-free, and inlining its callees is what makes it call-free, so the
/// candidate set is rebuilt each round: round one inlines the accessors into
/// `sum_via_ref`, round two inlines `sum_via_ref` into its caller.
pub fn run(prog: &mut Program) {
    let mut budget = BUDGET;
    for _ in 0..64 {
        let cands: Vec<Cand> = prog.fns.iter().filter_map(Cand::new).collect();
        if cands.is_empty() {
            break;
        }
        let before = budget;
        let mut inl = Inliner { cands, budget };
        for f in &mut prog.fns {
            inl.walk_fn(f);
        }
        budget = inl.budget;
        if budget == before {
            break;
        }
    }
}

/// A function eligible for inlining, in the form the pass splices in: a body
/// whose value is its tail, with any trailing `return` folded into that tail.
struct Cand {
    name: String,
    params: Vec<Type>,
    /// The body's value: `None` for a unit function.
    tail: Option<Expr>,
    stmts: Vec<Stmt>,
    param_slots: Vec<i32>,
    nslots: i32,
}

impl Cand {
    fn new(f: &FnDef) -> Option<Cand> {
        // A by-value `self` receiver is an aggregate copy; leave it to the call
        // path. `&self`/`&mut self` is one pointer and is inlined normally.
        if f.takes_self && !f.ref_self {
            return None;
        }
        if !f.ret.is_scalar() && f.ret != Type::Unit {
            return None;
        }
        if f.params.iter().any(|(_, t)| !t.is_scalar()) || f.params.len() > 6 {
            return None;
        }
        // Split off a trailing `return`, then require the rest to be
        // straight-line and call-free.
        let mut stmts = f.body.stmts.clone();
        let mut tail = f.body.tail.as_deref().cloned();
        if let Some(Stmt::Return(v)) = stmts.last() {
            if tail.is_some() {
                return None; // a tail after an unconditional return
            }
            tail = v.clone();
            stmts.pop();
        }
        if !stmts.iter().all(stmt_ok) || tail.as_ref().map_or(false, |e| !expr_ok(e)) {
            return None;
        }
        let n = stmts.iter().map(count_stmt).sum::<usize>()
            + tail.as_ref().map_or(0, count_expr);
        if n > MAX_NODES {
            return None;
        }
        Some(Cand {
            name: f.name.clone(),
            params: f.params.iter().map(|(_, t)| t.clone()).collect(),
            tail,
            stmts,
            param_slots: f.param_slots.clone(),
            nslots: f.nslots,
        })
    }
}

/// True when `s` is straight-line: no loop and no early exit.
fn stmt_ok(s: &Stmt) -> bool {
    match s {
        Stmt::Let { init, .. } => init.as_ref().map_or(true, expr_ok),
        Stmt::Assign { target, value } => expr_ok(target) && expr_ok(value),
        Stmt::Expr(e) => expr_ok(e),
        Stmt::While(..) | Stmt::Loop(..) | Stmt::For { .. } => false,
        Stmt::Return(_) | Stmt::Break | Stmt::Continue => false,
    }
}

/// True when `e` is a scalar, call-free expression. Aggregates are excluded, so
/// an inlined body never introduces a frame buffer whose layout the caller does
/// not know about; a call is excluded, so inlining a candidate never adds work
/// that could itself grow.
fn expr_ok(e: &Expr) -> bool {
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {
            true
        }
        Expr::Bin(_, a, b) => expr_ok(a) && expr_ok(b),
        Expr::Un(_, a) | Expr::Cast(a, _) => expr_ok(a),
        Expr::Field { base, .. } => expr_ok(base),
        Expr::Index(a, b) => expr_ok(a) && expr_ok(b),
        Expr::Deref { expr, .. } => expr_ok(expr),
        // A reference to an lvalue is fine; a temporary (slot >= 0) is an
        // aggregate buffer, which is excluded.
        Expr::AddrOf { expr, slot, .. } => *slot < 0 && expr_ok(expr),
        Expr::If(c, t, els) => {
            expr_ok(c) && block_ok(t) && els.as_ref().map_or(true, block_ok)
        }
        Expr::Block(b) => block_ok(b),
        // Calls, aggregate literals and payload-enum operations are excluded.
        _ => false,
    }
}

fn block_ok(b: &Block) -> bool {
    b.stmts.iter().all(stmt_ok) && b.tail.as_deref().map_or(true, expr_ok)
}

fn count_stmt(s: &Stmt) -> usize {
    match s {
        Stmt::Let { init, .. } => init.as_ref().map_or(0, count_expr),
        Stmt::Assign { target, value } => count_expr(target) + count_expr(value),
        Stmt::Expr(e) => count_expr(e),
        Stmt::While(c, b) => count_expr(c) + b.stmts.iter().map(count_stmt).sum::<usize>(),
        Stmt::Loop(b) => b.stmts.iter().map(count_stmt).sum::<usize>(),
        Stmt::For { start, end, body, .. } => {
            count_expr(start) + count_expr(end) + body.stmts.iter().map(count_stmt).sum::<usize>()
        }
        Stmt::Return(v) => v.as_ref().map_or(0, count_expr),
        Stmt::Break | Stmt::Continue => 0,
    }
}

/// Count the nodes of an expression. A block's own value is included: with the
/// inliner unwrapping a statement-free block into its tail, a nested block is
/// usually gone by now, but a block that kept statements still carries its tail
/// and must be counted, or a candidate's true size would be understated.
fn count_expr(e: &Expr) -> usize {
    let n = |x: &Box<Expr>| count_expr(x);
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => 1,
        Expr::Bin(_, a, b) => 1 + n(a) + n(b),
        Expr::Un(_, a) | Expr::Cast(a, _) => 1 + n(a),
        Expr::Field { base, .. } => 1 + n(base),
        Expr::Index(a, b) => 1 + n(a) + n(b),
        Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => 1 + n(expr),
        Expr::Call { args, .. } => 1 + args.iter().map(count_expr).sum::<usize>(),
        Expr::MethodCall { recv, args, .. } => 1 + n(recv) + args.iter().map(count_expr).sum::<usize>(),
        Expr::If(c, t, els) => {
            1 + n(c)
                + t.stmts.iter().map(count_stmt).sum::<usize>()
                + t.tail.as_deref().map_or(0, count_expr)
                + els.as_ref().map_or(0, |b| {
                    b.stmts.iter().map(count_stmt).sum::<usize>()
                        + b.tail.as_deref().map_or(0, count_expr)
                })
        }
        Expr::Block(b) => {
            1 + b.stmts.iter().map(count_stmt).sum::<usize>()
                + b.tail.as_deref().map_or(0, count_expr)
        }
        Expr::StructLit { fields, .. } => 1 + fields.iter().map(|f| count_expr(&f.value)).sum::<usize>(),
        Expr::EnumCtor { args, .. } => 1 + args.iter().map(count_expr).sum::<usize>(),
        Expr::ArrayLit(v, _) => 1 + n(v),
        Expr::ArrayList(v) => 1 + v.iter().map(count_expr).sum::<usize>(),
        Expr::VariantTest { scrutinee, .. } => 1 + n(scrutinee),
        Expr::EnumPayload { recv, .. } => 1 + n(recv),
    }
}

struct Inliner {
    cands: Vec<Cand>,
    budget: usize,
}

impl Inliner {
    /// Inline every eligible call in one function, then grow its frame to cover
    /// the slots the expansions introduced.
    ///
    /// A single pass suffices: a candidate body is call-free, so inlining it
    /// cannot expose a new call to the fixed candidate set of this round. A
    /// caller whose callee only becomes eligible later is handled by the next
    /// round, which rebuilds the set.
    fn walk_fn(&mut self, f: &mut FnDef) {
        // Fresh slots start above every slot sema allocated, so an expansion can
        // never alias a caller local.
        let mut base = f.nslots;
        let mut body = std::mem::replace(&mut f.body, Block { stmts: Vec::new(), tail: None });
        self.walk_block(&mut body, &mut base, &mut HashSet::new());
        f.body = body;
        f.nslots = base;
    }

    fn walk_block(&mut self, b: &mut Block, base: &mut i32, expanding: &mut HashSet<String>) {
        for st in &mut b.stmts {
            self.walk_stmt(st, base, expanding);
        }
        if let Some(t) = &mut b.tail {
            self.walk_expr(t, base, expanding);
        }
    }

    fn walk_stmt(&mut self, s: &mut Stmt, base: &mut i32, expanding: &mut HashSet<String>) {
        match s {
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    self.walk_expr(e, base, expanding);
                }
            }
            Stmt::Assign { target, value } => {
                self.walk_expr(target, base, expanding);
                self.walk_expr(value, base, expanding);
            }
            Stmt::Expr(e) => self.walk_expr(e, base, expanding),
            Stmt::While(c, b) => {
                self.walk_expr(c, base, expanding);
                self.walk_block(b, base, expanding);
            }
            Stmt::Loop(b) => self.walk_block(b, base, expanding),
            Stmt::For { start, end, body, .. } => {
                self.walk_expr(start, base, expanding);
                self.walk_expr(end, base, expanding);
                self.walk_block(body, base, expanding);
            }
            Stmt::Return(v) => {
                if let Some(e) = v {
                    self.walk_expr(e, base, expanding);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn walk_expr(&mut self, e: &mut Expr, base: &mut i32, expanding: &mut HashSet<String>) {
        if self.try_inline(e, base, expanding) {
            // `try_inline` already walked the spliced body, which is where any
            // further calls now live.
            return;
        }
        match e {
            Expr::Bin(_, a, b) => {
                self.walk_expr(a, base, expanding);
                self.walk_expr(b, base, expanding);
            }
            Expr::Un(_, a) | Expr::Cast(a, _) => self.walk_expr(a, base, expanding),
            Expr::Field { base: b, .. } => self.walk_expr(b, base, expanding),
            Expr::Index(a, b) => {
                self.walk_expr(a, base, expanding);
                self.walk_expr(b, base, expanding);
            }
            Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => {
                self.walk_expr(expr, base, expanding)
            }
            Expr::If(c, t, els) => {
                self.walk_expr(c, base, expanding);
                self.walk_block(t, base, expanding);
                if let Some(b) = els {
                    self.walk_block(b, base, expanding);
                }
            }
            Expr::Block(b) => self.walk_block(b, base, expanding),
            Expr::Call { args, .. } => {
                for a in args {
                    self.walk_expr(a, base, expanding);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                self.walk_expr(recv, base, expanding);
                for a in args {
                    self.walk_expr(a, base, expanding);
                }
            }
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    self.walk_expr(&mut f.value, base, expanding);
                }
            }
            Expr::EnumCtor { args, .. } => {
                for a in args {
                    self.walk_expr(a, base, expanding);
                }
            }
            Expr::ArrayLit(v, _) => self.walk_expr(v, base, expanding),
            Expr::ArrayList(v) => {
                for x in v {
                    self.walk_expr(x, base, expanding);
                }
            }
            Expr::VariantTest { scrutinee, .. } => self.walk_expr(scrutinee, base, expanding),
            Expr::EnumPayload { recv, .. } => self.walk_expr(recv, base, expanding),
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }

    fn try_inline(
        &mut self,
        e: &mut Expr,
        base: &mut i32,
        expanding: &mut HashSet<String>,
    ) -> bool {
        // Decide eligibility before touching the expression, so a call that is
        // not inlined is left exactly as it was.
        let (name, nargs) = match e {
            Expr::Call { name, args, .. } => (name.clone(), args.len()),
            _ => return false,
        };
        if self.budget == 0 || expanding.contains(&name) {
            return false;
        }
        let cand = match self.cands.iter().position(|c| c.name == name) {
            Some(i) => i,
            None => return false,
        };
        if self.cands[cand].params.len() != nargs {
            return false;
        }

        let mut args = match e {
            Expr::Call { args, .. } => std::mem::take(args),
            _ => unreachable!(),
        };
        // Inline into each argument first, then bind it. Arguments are moved
        // into the expansion, so they are not walked again afterwards.
        for a in &mut args {
            self.walk_expr(a, base, expanding);
        }

        let region = self.cands[cand].nslots;
        let shift = *base;
        *base += region;
        let param_slots = self.cands[cand].param_slots.clone();
        let param_tys = self.cands[cand].params.clone();
        let assigned = assigned_slots(&self.cands[cand].stmts, self.cands[cand].tail.as_ref());

        // Bind each argument to what the body should read for its parameter. A
        // trivial argument (a name, a literal, a string) is substituted in
        // place; anything else may have side effects or need evaluating, so it
        // is evaluated once into a fresh slot and the slot is substituted, which
        // also keeps the evaluation order left to right.
        //
        // A parameter the body assigns is a special case: it is a local copy, so
        // substituting a trivial argument for it would make the assignment write
        // through to the caller's variable. Such a parameter is always copied
        // into a fresh slot, and the body's assignment targets that slot.
        let mut binds: Vec<Stmt> = Vec::new();
        let mut subst: Vec<Expr> = Vec::with_capacity(args.len());
        for (i, arg) in args.into_iter().enumerate() {
            if trivial(&arg) && !assigned.contains(&param_slots[i]) {
                subst.push(arg);
            } else {
                let slot = *base;
                *base += 1;
                binds.push(Stmt::Let {
                    name: format!("\u{0}arg{i}"),
                    ty: Some(param_tys[i].clone()),
                    init: Some(arg),
                    slot,
                });
                subst.push(Expr::Var { name: format!("\u{0}arg{i}"), slot, ty: param_tys[i].clone() });
            }
        }

        // Substitute each bound argument for every read of its parameter's slot
        // while shifting the callee's other slots into the caller's frame.
        let nbinds = binds.len();
        let mut stmts = binds;
        stmts.extend(self.cands[cand].stmts.clone());
        let mut tail = self.cands[cand].tail.clone();
        let mut sub = Sub { shift, param_slots: &param_slots, args: &subst };
        for st in &mut stmts[nbinds..] {
            sub.stmt(st);
        }
        if let Some(t) = &mut tail {
            sub.expr(t);
        }

        self.budget -= 1;
        expanding.insert(name.clone());
        let mut block = Block { stmts, tail: tail.map(Box::new) };
        self.walk_block(&mut block, base, expanding);
        expanding.remove(&name);

        // A block with no statements is just its value, and unwrapping it lets
        // later passes see through it: `refs`'s hot loop reads `acc + w.total()
        // + ...`, and the inlined accessors would otherwise stay wrapped in
        // blocks that the accumulator fold cannot inspect, forcing the whole sum
        // through `%rax` and the spill slots every iteration.
        *e = if block.stmts.is_empty() && block.tail.is_some() {
            *block.tail.take().unwrap()
        } else {
            Expr::Block(block)
        };
        true
    }
}

/// True for an argument that can be substituted into a body without changing
/// evaluation: a name, a literal, or a string. Everything else is evaluated once
/// into a temporary instead.
fn trivial(e: &Expr) -> bool {
    matches!(e, Expr::Var { .. } | Expr::Int(_) | Expr::Bool(_) | Expr::Str(_))
}

/// The slots a candidate body assigns directly (not through a pointer or an
/// element). A parameter among them is a local the body mutates, so it must be
/// copied rather than substituted, or the mutation would reach the caller.
fn assigned_slots(stmts: &[Stmt], tail: Option<&Expr>) -> HashSet<i32> {
    fn collect_block(b: &Block, out: &mut HashSet<i32>) {
        for s in &b.stmts {
            collect_stmt(s, out);
        }
        if let Some(t) = &b.tail {
            collect_expr(t, out);
        }
    }
    fn collect_stmt(s: &Stmt, out: &mut HashSet<i32>) {
        match s {
            Stmt::Assign { target, value } => {
                if let Expr::Var { slot, .. } = target {
                    out.insert(*slot);
                }
                collect_expr(target, out);
                collect_expr(value, out);
            }
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    collect_expr(e, out);
                }
            }
            Stmt::Expr(e) => collect_expr(e, out),
            Stmt::While(c, b) => {
                collect_expr(c, out);
                collect_block(b, out);
            }
            Stmt::Loop(b) => collect_block(b, out),
            Stmt::For { start, end, body, var_slot, end_slot, .. } => {
                out.insert(*var_slot);
                out.insert(*end_slot);
                collect_expr(start, out);
                collect_expr(end, out);
                collect_block(body, out);
            }
            Stmt::Return(v) => {
                if let Some(e) = v {
                    collect_expr(e, out);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }
    fn collect_expr(e: &Expr, out: &mut HashSet<i32>) {
        match e {
            Expr::Bin(_, a, b) => {
                collect_expr(a, out);
                collect_expr(b, out);
            }
            Expr::Un(_, a) | Expr::Cast(a, _) => collect_expr(a, out),
            Expr::Field { base, .. } => collect_expr(base, out),
            Expr::Index(a, b) => {
                collect_expr(a, out);
                collect_expr(b, out);
            }
            Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => collect_expr(expr, out),
            Expr::Block(b) => collect_block(b, out),
            Expr::If(c, t, els) => {
                collect_expr(c, out);
                collect_block(t, out);
                if let Some(b) = els {
                    collect_block(b, out);
                }
            }
            Expr::Call { args, .. } => {
                for a in args {
                    collect_expr(a, out);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                collect_expr(recv, out);
                for a in args {
                    collect_expr(a, out);
                }
            }
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    collect_expr(&f.value, out);
                }
            }
            Expr::EnumCtor { args, .. } => {
                for a in args {
                    collect_expr(a, out);
                }
            }
            Expr::ArrayLit(v, _) => collect_expr(v, out),
            Expr::ArrayList(v) => {
                for x in v {
                    collect_expr(x, out);
                }
            }
            Expr::VariantTest { scrutinee, .. } => collect_expr(scrutinee, out),
            Expr::EnumPayload { recv, .. } => collect_expr(recv, out),
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }

    let mut out = HashSet::new();
    for s in stmts {
        collect_stmt(s, &mut out);
    }
    if let Some(t) = tail {
        collect_expr(t, &mut out);
    }
    out
}

/// Rewrites a callee body into the caller's frame: every slot is shifted by
/// `shift`, and a read of parameter `i`'s slot is replaced by argument `i`.
struct Sub<'a> {
    shift: i32,
    param_slots: &'a [i32],
    args: &'a [Expr],
}

impl Sub<'_> {
    fn slot(&self, s: i32) -> i32 {
        self.shift + s
    }

    /// The argument bound to slot `s` if `s` is a parameter slot.
    fn param_arg(&self, s: i32) -> Option<Expr> {
        self.param_slots.iter().position(|p| *p == s).map(|i| self.args[i].clone())
    }

    fn block(&mut self, b: &mut Block) {
        for st in &mut b.stmts {
            self.stmt(st);
        }
        if let Some(t) = &mut b.tail {
            self.expr(t);
        }
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { init, slot, .. } => {
                *slot = self.slot(*slot);
                if let Some(e) = init {
                    self.expr(e);
                }
            }
            Stmt::Assign { target, value } => {
                self.expr(target);
                self.expr(value);
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::While(c, b) => {
                self.expr(c);
                self.block(b);
            }
            Stmt::Loop(b) => self.block(b),
            Stmt::For { start, end, body, var_slot, end_slot, .. } => {
                *var_slot = self.slot(*var_slot);
                *end_slot = self.slot(*end_slot);
                self.expr(start);
                self.expr(end);
                self.block(body);
            }
            Stmt::Return(v) => {
                if let Some(e) = v {
                    self.expr(e);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        // A parameter read is replaced wholesale by its argument; the argument
        // already lives in the caller's frame, so it is not shifted again.
        if let Expr::Var { slot, .. } = e {
            if let Some(arg) = self.param_arg(*slot) {
                *e = arg;
                return;
            }
        }
        match e {
            Expr::Var { slot, .. } => *slot = self.slot(*slot),
            Expr::Bin(_, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Un(_, a) | Expr::Cast(a, _) => self.expr(a),
            Expr::Field { base, .. } => self.expr(base),
            Expr::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Deref { expr, .. } => self.expr(expr),
            Expr::AddrOf { expr, slot, .. } => {
                if *slot >= 0 {
                    *slot = self.slot(*slot);
                }
                self.expr(expr);
            }
            Expr::If(c, t, els) => {
                self.expr(c);
                self.block(t);
                if let Some(b) = els {
                    self.block(b);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::Call { args, arg_slots, ret_slot, .. } => {
                for (s, _) in arg_slots.iter_mut() {
                    if *s >= 0 {
                        *s = self.slot(*s);
                    }
                }
                if *ret_slot >= 0 {
                    *ret_slot = self.slot(*ret_slot);
                }
                for a in args {
                    self.expr(a);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                self.expr(recv);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::StructLit { fields, slot, .. } => {
                *slot = self.slot(*slot);
                for f in fields {
                    self.expr(&mut f.value);
                }
            }
            Expr::EnumCtor { args, slot, .. } => {
                *slot = self.slot(*slot);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::ArrayLit(v, _) => self.expr(v),
            Expr::ArrayList(v) => {
                for x in v {
                    self.expr(x);
                }
            }
            Expr::VariantTest { scrutinee, .. } => self.expr(scrutinee),
            Expr::EnumPayload { recv, .. } => self.expr(recv),
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::EnumVariant { .. } => {}
        }
    }
}
