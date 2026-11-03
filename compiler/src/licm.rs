//! Loop-invariant code motion, run on the AST after inlining and before code
//! generation.
//!
//! Inlining leaves accessor arithmetic inside the loop that reads only
//! loop-invariant data. The `refs` hot loop reads a fixed struct through three
//! accessors every iteration; after inlining, the whole sum of the struct's
//! fields is invariant, yet it is recomputed each pass. This pass computes each
//! such subexpression once, before the loop, and reads the result inside.
//!
//! The pass is deliberately conservative, because hoisting across an iteration
//! boundary is easy to get wrong:
//!
//! * It only hoists a *pure* scalar expression: no call, no division or modulo
//!   (which can trap), no array index and no dereference (both can read memory
//!   an alias could change), and no block or `if`. Struct-field reads are
//!   allowed, but only when the loop writes no memory at all.
//! * Every local the expression reads must be untouched by the loop. If the loop
//!   contains any call, the pass gives up on that loop entirely, since a call
//!   can write anything.
//! * It is disabled under `--overflow-checks`: a hoisted `+`/`-`/`*` would trap
//!   before the loop instead of at the iteration that overflows, which is an
//!   observable change.
//!
//! Hoisting a pure, non-trapping value earlier is otherwise always safe: it can
//! only waste a computation when the loop does not run, never change a result.
//!
//! Only a loop *body* is rewritten. A loop's condition and a `for`'s bounds run
//! once per iteration by design, and hoisting them would change the loop.

use crate::ast::*;
use std::collections::{HashMap, HashSet};

/// Reject candidates smaller than this many nodes: a bare local or literal is
/// already one load, so a temporary for it only adds a move.
const MIN_NODES: usize = 2;

/// Run LICM over every function of `prog`. `overflow_checks` disables the pass,
/// since a hoisted arithmetic trap would fire at the wrong time.
pub fn run(prog: &mut Program, overflow_checks: bool) {
    if overflow_checks {
        return;
    }
    for f in &mut prog.fns {
        let mut l = Licm { nslots: f.nslots };
        l.block(&mut f.body);
        f.nslots = l.nslots;
    }
}

struct Licm {
    nslots: i32,
}

impl Licm {
    fn block(&mut self, b: &mut Block) {
        let stmts = std::mem::take(&mut b.stmts);
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            self.stmt(s, &mut out);
        }
        b.stmts = out;
        if let Some(t) = &mut b.tail {
            self.expr_loops(t);
        }
    }

    /// Process one statement, appending any hoist bindings (which must run
    /// before it) followed by the statement itself to `out`.
    fn stmt(&mut self, mut s: Stmt, out: &mut Vec<Stmt>) {
        match &mut s {
            Stmt::While(c, body) => {
                self.expr_loops(c);
                self.block(body);
                // The condition runs before every iteration, so its effects are
                // part of the loop's.
                let mut eff = Effects::of_block(body);
                eff.expr(c);
                let binds = self.hoist_loop(body, &[], eff);
                out.extend(binds);
            }
            Stmt::Loop(body) => {
                self.block(body);
                let binds = self.hoist_loop(body, &[], Effects::of_block(body));
                out.extend(binds);
            }
            Stmt::For { start, end, body, var_slot, end_slot, .. } => {
                self.expr_loops(start);
                self.expr_loops(end);
                self.block(body);
                // The loop variable is assigned every iteration, and the bound
                // is a slot the `for` itself writes; both live on the `For` node
                // rather than in the body, so they are handed to the effect scan
                // explicitly. The start and end expressions run once before the
                // loop, but they can still call a function, so their effects must
                // be included or a read of a variable they mutate looks invariant.
                let mut eff = Effects::of_block(body);
                eff.expr(start);
                eff.expr(end);
                let binds = self.hoist_loop(body, &[*var_slot, *end_slot], eff);
                out.extend(binds);
            }
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    self.expr_loops(e);
                }
            }
            Stmt::Assign { target, value } => {
                self.expr_loops(target);
                self.expr_loops(value);
            }
            Stmt::Expr(e) => self.expr_loops(e),
            Stmt::Return(v) => {
                if let Some(e) = v {
                    self.expr_loops(e);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
        out.push(s);
    }

    /// Find loops nested inside an expression (in a block or an `if` arm) and
    /// process them. A loop is a statement, so it only appears inside a block.
    fn expr_loops(&mut self, e: &mut Expr) {
        match e {
            Expr::Block(b) => self.block(b),
            Expr::MatchBoundary(b, _) => self.block(b),
            Expr::If(c, t, els) => {
                self.expr_loops(c);
                self.block(t);
                if let Some(b) = els {
                    self.block(b);
                }
            }
            Expr::Bin(_, a, b) => {
                self.expr_loops(a);
                self.expr_loops(b);
            }
            Expr::Un(_, a) | Expr::Cast(a, _) => self.expr_loops(a),
            Expr::Field { base, .. } => self.expr_loops(base),
            Expr::Index(a, b) => {
                self.expr_loops(a);
                self.expr_loops(b);
            }
            Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => self.expr_loops(expr),
            Expr::Call { args, .. } => {
                for a in args {
                    self.expr_loops(a);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                self.expr_loops(recv);
                for a in args {
                    self.expr_loops(a);
                }
            }
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    self.expr_loops(&mut f.value);
                }
            }
            Expr::EnumCtor { args, .. } => {
                for a in args {
                    self.expr_loops(a);
                }
            }
            Expr::ArrayLit(v, _) => self.expr_loops(v),
            Expr::ArrayList(v) => {
                for x in v {
                    self.expr_loops(x);
                }
            }
            Expr::VariantTest { scrutinee, .. } => self.expr_loops(scrutinee),
            Expr::EnumPayload { recv, .. } => self.expr_loops(recv),
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }

    /// Hoist the invariant subexpressions of one loop body, returning the
    /// bindings to run before the loop. `carried` names slots the loop itself
    /// assigns outside the body (a `for`'s variable and bound), and `eff` is the
    /// already-computed effect of the body plus the loop's condition or bounds.
    fn hoist_loop(&mut self, body: &mut Block, carried: &[i32], mut eff: Effects) -> Vec<Stmt> {
        if eff.all {
            return Vec::new();
        }
        for s in carried {
            eff.slots.insert(*s);
        }
        let mut h = Hoister { nslots: self.nslots, eff, binds: Vec::new(), seen: HashMap::new() };
        h.block(body);
        self.nslots = h.nslots;
        h.binds
    }
}

/// What a loop body does, as far as invariance needs to know: which locals it
/// assigns, whether it writes memory, and whether it calls anything (in which
/// case nothing can be assumed invariant).
struct Effects {
    slots: HashSet<i32>,
    mem: bool,
    all: bool,
}

impl Effects {
    fn of_block(b: &Block) -> Effects {
        let mut e = Effects { slots: HashSet::new(), mem: false, all: false };
        e.block(b);
        e
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { slot, init, .. } => {
                self.slots.insert(*slot);
                if let Some(e) = init {
                    self.expr(e);
                }
            }
            Stmt::Assign { target, value } => {
                match target {
                    Expr::Var { slot, ty, .. } => {
                        self.slots.insert(*slot);
                        // Assigning a whole aggregate writes several words, which
                        // a reference to it observes; the slot alone does not
                        // cover a read through a pointer to the struct.
                        if ty.is_aggregate() {
                            self.mem = true;
                        }
                    }
                    // A store through a field, element or pointer writes memory
                    // the pass cannot track.
                    _ => self.mem = true,
                }
                // The target is itself an expression: an index or a dereference
                // can call a function that mutates a local through a reference.
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
                self.slots.insert(*var_slot);
                self.slots.insert(*end_slot);
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

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Call { args, .. } => {
                // A call may write anything, so no local is known invariant.
                self.all = true;
                for a in args {
                    self.expr(a);
                }
            }
            Expr::MethodCall { recv, args, .. } => {
                self.all = true;
                self.expr(recv);
                for a in args {
                    self.expr(a);
                }
            }
            // Taking an address can let a callee — or a later dereference —
            // write through it; be safe. A local whose address is taken may also
            // be written through that pointer, so it is no longer invariant even
            // if nothing assigns it directly. This is what catches `&mut x`
            // passed to a function that was inlined into the loop: the call is
            // gone, but the deref store it became still writes `x`.
            Expr::AddrOf { expr, .. } => {
                self.mem = true;
                if let Expr::Var { slot, .. } = expr.as_ref() {
                    self.slots.insert(*slot);
                }
                self.expr(expr);
            }
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
            Expr::Block(b) => self.block(b),
            Expr::MatchBoundary(b, _) => self.block(b),
            Expr::If(c, t, els) => {
                self.expr(c);
                self.block(t);
                if let Some(b) = els {
                    self.block(b);
                }
            }
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    self.expr(&f.value);
                }
            }
            Expr::EnumCtor { args, .. } => {
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
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }
}

struct Hoister {
    nslots: i32,
    eff: Effects,
    binds: Vec<Stmt>,
    seen: HashMap<String, (i32, Type)>,
}

impl Hoister {
    fn block(&mut self, b: &mut Block) {
        for s in &mut b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &mut b.tail {
            self.expr(t);
        }
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    self.expr(e);
                }
            }
            Stmt::Assign { value, .. } => self.expr(value),
            Stmt::Expr(e) => self.expr(e),
            Stmt::While(c, b) => {
                self.expr(c);
                self.block(b);
            }
            Stmt::Loop(b) => self.block(b),
            Stmt::For { start, end, body, .. } => {
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
        if self.invariant(e) && count_expr(e) >= MIN_NODES && scalar(e) {
            let key = fmt_expr(e);
            if let Some((slot, ty)) = self.seen.get(&key).cloned() {
                *e = Expr::Var { name: format!("\u{0}licm{slot}"), slot, ty };
            } else {
                let slot = self.nslots;
                self.nslots += 1;
                let ty = expr_ty(e);
                let value = e.clone();
                self.binds.push(Stmt::Let {
                    name: format!("\u{0}licm{slot}"),
                    ty: Some(ty.clone()),
                    init: Some(value),
                    slot,
                });
                self.seen.insert(key, (slot, ty.clone()));
                *e = Expr::Var { name: format!("\u{0}licm{slot}"), slot, ty };
            }
            return;
        }
        match e {
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
            Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => self.expr(expr),
            Expr::Block(b) => self.block(b),
            Expr::MatchBoundary(b, _) => self.block(b),
            Expr::If(c, t, els) => {
                self.expr(c);
                self.block(t);
                if let Some(b) = els {
                    self.block(b);
                }
            }
            Expr::Call { args, .. } => {
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
            Expr::StructLit { fields, .. } => {
                for f in fields {
                    self.expr(&mut f.value);
                }
            }
            Expr::EnumCtor { args, .. } => {
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
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }

    /// True when `e` reads only loop-invariant data and cannot trap or write.
    fn invariant(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Bool(_) | Expr::EnumVariant { .. } => true,
            Expr::Var { slot, .. } => !self.eff.all && !self.eff.slots.contains(slot),
            // A field read is memory, so it is only invariant when the loop
            // writes no memory at all.
            Expr::Field { base, ty, .. } => {
                !self.eff.mem && ty.is_scalar() && self.invariant(base)
            }
            Expr::Bin(op, a, b) => {
                !matches!(op, BinOp::Div | BinOp::Mod) && self.invariant(a) && self.invariant(b)
            }
            Expr::Un(_, a) => self.invariant(a),
            Expr::Cast(a, _) => self.invariant(a),
            // Index, Deref, Call, AddrOf, blocks and aggregates are never hoisted.
            _ => false,
        }
    }
}

/// True when `e` is a scalar value (integer, boolean or reference).
fn scalar(e: &Expr) -> bool {
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::EnumVariant { .. } => true,
        Expr::Var { ty, .. } => ty.is_scalar(),
        Expr::Field { ty, .. } => ty.is_scalar(),
        Expr::Un(UnOp::Neg | UnOp::Not, a) => scalar(a),
        Expr::Bin(op, a, b) => !matches!(op, BinOp::Div | BinOp::Mod) && scalar(a) && scalar(b),
        _ => false,
    }
}

/// The type of a hoistable expression: `Bool` for a comparison or `!`, `I64`
/// otherwise.
fn expr_ty(e: &Expr) -> Type {
    match e {
        Expr::Bool(_) => Type::Bool,
        Expr::Un(UnOp::Not, _) => Type::Bool,
        Expr::Bin(op, _, _)
            if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) =>
        {
            Type::Bool
        }
        Expr::Bin(_, a, _) if matches!(a.as_ref(), Expr::Bool(_)) => Type::Bool,
        _ => Type::I64,
    }
}

/// A structural key so the same invariant expression is hoisted once, however
/// many times it appears.
fn fmt_expr(e: &Expr) -> String {
    match e {
        Expr::Int(v) => format!("#{v}"),
        Expr::Bool(b) => format!("#{}", if *b { 1 } else { 0 }),
        Expr::Var { slot, .. } => format!("${slot}"),
        Expr::Bin(op, a, b) => format!("({:?} {} {})", op, fmt_expr(a), fmt_expr(b)),
        Expr::Un(op, a) => format!("({:?} {})", op, fmt_expr(a)),
        Expr::Cast(a, _) => format!("(cast {})", fmt_expr(a)),
        Expr::Field { base, offset, .. } => format!("(f{} {})", offset, fmt_expr(base)),
        Expr::EnumVariant { enum_name, variant, .. } => format!("{enum_name}::{variant}"),
        _ => "\u{0}opaque".into(),
    }
}

fn count_expr(e: &Expr) -> usize {
    let n = |x: &Box<Expr>| count_expr(x);
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => 1,
        Expr::Bin(_, a, b) => 1 + n(a) + n(b),
        Expr::Un(_, a) | Expr::Cast(a, _) => 1 + n(a),
        Expr::Field { base, .. } => 1 + n(base),
        _ => 1_000_000,
    }
}
