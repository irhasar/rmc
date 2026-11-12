//! Loop-invariant code motion, run on the AST after inlining and before code
//! generation.
//!
//! Inlining leaves accessor arithmetic inside the loop that reads only
//! loop-invariant data. A `for`/`while` loop that sums a fixed struct's fields
//! recomputes the whole sum each pass; after inlining the sum is invariant, so
//! this pass computes it once, before the loop, and reads the result inside.
//!
//! The pass is deliberately conservative, because hoisting across an iteration
//! boundary is easy to get wrong:
//!
//! * It only hoists a *pure* scalar expression: no call, no division or modulo
//!   (which can trap), no array index and no dereference (both can read memory
//!   an alias could change), and no block or `if`. Struct-field reads are
//!   allowed, but only when the loop writes no memory at all.
//! * A field read through a *reference* (`p.x` where `p: &T`, or `(*q).x`) is
//!   hoisted only when the reference is *proven* to point at a live local of
//!   this function (`let p = &local;`, or a copy of such a reference). Such a
//!   pointer cannot be null or dangle, so speculating the read cannot fault,
//!   and the loop writes no memory, so the value is unchanged. Any other
//!   reference — a parameter, a field, a dereference of one, or one whose
//!   operand is not a frame place — is unproven and its reads stay in the loop,
//!   because rmc lets a reference outlive its referent (`0 as &i64` is accepted,
//!   and a `&` to a local can be returned) and dereferencing one is a memory
//!   read that can fault. Hoisting it would fault on a loop that never runs,
//!   where the read inside the body would not.
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
        let proven = proven_refs(&f.body);
        let mut l = Licm { nslots: f.nslots, proven };
        l.block(&mut f.body);
        f.nslots = l.nslots;
    }
}

struct Licm {
    nslots: i32,
    /// Slots holding a reference proven to point at a live local of this
    /// function, so a read through one cannot fault (see `proven_refs`).
    proven: HashSet<i32>,
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
            Stmt::While(c, body, _) => {
                self.expr_loops(c);
                self.block(body);
                // The condition runs before every iteration, so its effects are
                // part of the loop's.
                let mut eff = Effects::of_block(body);
                eff.expr(c);
                let binds = self.hoist_loop(body, &[], eff);
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
            Stmt::Break(v, _) => {
                if let Some(e) = v {
                    self.expr_loops(e);
                }
            }
            Stmt::Continue(_) => {}
        }
        out.push(s);
    }

    /// Find loops nested inside an expression (in a block or an `if` arm) and
    /// process them. A loop is a statement, so it only appears inside a block.
    fn expr_loops(&mut self, e: &mut Expr) {
        match e {
Expr::PatScrutinee(_) => {
                unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
            }
            Expr::PatCheck { .. } => unreachable!("sema rewrites PatCheck into `true`"),
            Expr::PatArity { .. } => unreachable!("sema rewrites PatArity into `true`"),
            Expr::Block(b) | Expr::Loop(b, _, _, _) => self.block(b),
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
            Expr::TupleLit(v) => {
                for x in v {
                    self.expr_loops(x);
                }
            }
            Expr::VariantTest { scrutinee, .. } => self.expr_loops(scrutinee),
            Expr::BoolTest { scrutinee, .. } => self.expr_loops(scrutinee),
            Expr::EnumPayload { recv, .. } => self.expr_loops(recv),
            Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
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
        let mut h = Hoister {
            nslots: self.nslots,
            eff,
            binds: Vec::new(),
            seen: HashMap::new(),
            proven: self.proven.clone(),
        };
        h.block(body);
        self.nslots = h.nslots;
        h.binds
    }
}

/// The slots of `body` that hold a reference *proven* to point at a live local
/// of this function, so a read through one cannot fault.
///
/// A slot is proven when an immutable `let` binds it to `&place` for a frame
/// place (`&local` or `&local.field`), or to another proven slot. The operand
/// must be a local the function owns, so the pointer is to live frame storage
/// and can neither be null nor dangle while the reference is live. An immutable
/// binding cannot be reassigned (sema rejects assigning a non-`mut` local), so
/// the proof holds for the binding's whole life; a `mut` reference is not
/// proven. Every other reference — a parameter, a struct field, a dereference of
/// one, `&*p`, or a cast from an integer — is unproven: rmc lets it be null or
/// dangling (`0 as &i64`, or a returned `&local`), so a speculative read through
/// it could fault.
///
/// The set is a fixed point over the whole body, not just the loop: `let p =
/// &x;` after a loop must be seen for a `let q = p;` copy read inside it to be
/// proven, and the source may be walked after the copy. It is *may*-proven (a
/// slot added by any binding is kept), which is sound because a proven slot is
/// an immutable binding: nothing can later make it hold a different pointer.
fn proven_refs(body: &Block) -> HashSet<i32> {
    let mut proven = HashSet::new();
    loop {
        let before = proven.len();
        scan_proven(body, &mut proven);
        if proven.len() == before {
            return proven;
        }
    }
}

/// True when `e` builds a reference to a frame place: `&local` or
/// `&local.field`. The operand must be a place this function owns, so the
/// pointer cannot be null or dangle.
fn is_frame_ref(e: &Expr) -> bool {
    matches!(e, Expr::AddrOf { expr, .. } if is_frame_place(expr))
}

/// True when `e` names a frame place: a local, or a non-reference field of one.
/// A reference-typed field is a pointer, so a place past it is not a frame
/// place. Nor is a field whose *base* is a reference: `r.inner` where `r: &S`
/// lives at `*r + offset`, so forming its address dereferences `r`, which may be
/// null or dangle. Only a field reached through a real in-frame aggregate
/// (`&local.field`) is frame-owned. `&r` itself stays a frame place — it is the
/// address of the local pointer slot — which is why the base check is on the
/// field arm rather than on `Var`.
fn is_frame_place(e: &Expr) -> bool {
    match e {
        Expr::Var { .. } => true,
        Expr::Field { base, ty, .. } => {
            !matches!(ty, Type::Ref(..)) && !is_ref_typed(base) && is_frame_place(base)
        }
        _ => false,
    }
}

/// True when `e`'s type is a reference, so reaching a field or element through
/// it dereferences a pointer rather than naming a frame place.
fn is_ref_typed(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Var { ty, .. } | Expr::Field { ty, .. } | Expr::Deref { ty, .. }
            if matches!(ty, Type::Ref(..))
    )
}

/// Record the destination of an immutable `let` that is bound to a proven
/// reference, and recurse through every statement and block.
fn scan_proven(b: &Block, proven: &mut HashSet<i32>) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init: Some(e), slot, mutable: false, .. } => {
                if is_frame_ref(e) || matches!(e, Expr::Var { slot, .. } if proven.contains(slot)) {
                    proven.insert(*slot);
                }
                scan_expr_proven(e, proven);
            }
            Stmt::Let { init: Some(e), .. } => scan_expr_proven(e, proven),
            Stmt::Let { init: None, .. } => {}
            Stmt::Assign { value, .. } => scan_expr_proven(value, proven),
            Stmt::Expr(e) => scan_expr_proven(e, proven),
            Stmt::While(c, body, _) => {
                scan_expr_proven(c, proven);
                scan_proven(body, proven);
            }
            Stmt::For { start, end, body, .. } => {
                scan_expr_proven(start, proven);
                scan_expr_proven(end, proven);
                scan_proven(body, proven);
            }
            Stmt::Return(Some(e)) | Stmt::Break(Some(e), _) => scan_expr_proven(e, proven),
            Stmt::Return(None) | Stmt::Break(None, _) | Stmt::Continue(_) => {}
        }
    }
    if let Some(t) = &b.tail {
        scan_expr_proven(t, proven);
    }
}

/// Recurse into the sub-expressions of `e` so a `let` inside a nested block or
/// `if` arm is seen.
fn scan_expr_proven(e: &Expr, proven: &mut HashSet<i32>) {
    match e {
        Expr::Block(b) | Expr::Loop(b, _, _, _) | Expr::MatchBoundary(b, _) => scan_proven(b, proven),
        Expr::If(c, t, els) => {
            scan_expr_proven(c, proven);
            scan_proven(t, proven);
            if let Some(b) = els {
                scan_proven(b, proven);
            }
        }
        Expr::Bin(_, a, b) => {
            scan_expr_proven(a, proven);
            scan_expr_proven(b, proven);
        }
        Expr::Un(_, a) | Expr::Cast(a, _) => scan_expr_proven(a, proven),
        Expr::Field { base, .. } => scan_expr_proven(base, proven),
        Expr::Index(a, b) => {
            scan_expr_proven(a, proven);
            scan_expr_proven(b, proven);
        }
        Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => scan_expr_proven(expr, proven),
        Expr::Call { args, .. } => {
            for a in args {
                scan_expr_proven(a, proven);
            }
        }
        Expr::MethodCall { recv, args, .. } => {
            scan_expr_proven(recv, proven);
            for a in args {
                scan_expr_proven(a, proven);
            }
        }
        Expr::StructLit { fields, .. } => {
            for f in fields {
                scan_expr_proven(&f.value, proven);
            }
        }
        Expr::EnumCtor { args, .. } => {
            for a in args {
                scan_expr_proven(a, proven);
            }
        }
        Expr::ArrayLit(v, _) => scan_expr_proven(v, proven),
        Expr::ArrayList(v) | Expr::TupleLit(v) => {
            for x in v {
                scan_expr_proven(x, proven);
            }
        }
        Expr::VariantTest { scrutinee, .. } | Expr::BoolTest { scrutinee, .. } => {
            scan_expr_proven(scrutinee, proven);
        }
        Expr::EnumPayload { recv, .. } => scan_expr_proven(recv, proven),
        Expr::PatScrutinee(_) | Expr::PatCheck { .. } | Expr::PatArity { .. } => {}
        Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
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
            Stmt::While(c, b, _) => {
                self.expr(c);
                self.block(b);
            }
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
            Stmt::Break(v, _) => {
                if let Some(e) = v {
                    self.expr(e);
                }
            }
            Stmt::Continue(_) => {}
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
Expr::PatScrutinee(_) => {
                unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
            }
            Expr::PatCheck { .. } => unreachable!("sema rewrites PatCheck into `true`"),
            Expr::PatArity { .. } => unreachable!("sema rewrites PatArity into `true`"),
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
            Expr::Block(b) | Expr::Loop(b, _, _, _) => self.block(b),
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
            Expr::TupleLit(v) => {
                for x in v {
                    self.expr(x);
                }
            }
            Expr::VariantTest { scrutinee, .. } => self.expr(scrutinee),
            Expr::BoolTest { scrutinee, .. } => self.expr(scrutinee),
            Expr::EnumPayload { recv, .. } => self.expr(recv),
            Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }
}

struct Hoister {
    nslots: i32,
    eff: Effects,
    binds: Vec<Stmt>,
    seen: HashMap<String, (i32, Type)>,
    /// The function's proven-reference slots, copied from `Licm` so the same
    /// `frame_field_base` rule decides what a hoist may speculate.
    proven: HashSet<i32>,
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
            Stmt::While(c, b, _) => {
                self.expr(c);
                self.block(b);
            }
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
            Stmt::Break(v, _) => {
                if let Some(e) = v {
                    self.expr(e);
                }
            }
            Stmt::Continue(_) => {}
        }
    }

    /// Rewrite `e`, if it is a chain of `+`, so that its invariant terms are
    /// grouped into one sub-sum. `+` is the only arithmetic the codegen folds
    /// into an in-place chain, so it is the only one worth regrouping.
    fn regroup_sum(&mut self, e: &mut Expr) {
        if !matches!(e, Expr::Bin(BinOp::Add, _, _)) {
            return;
        }
        let terms = collect_add_terms(e);
        if terms.len() < 2 {
            return;
        }
        let (mut inv, mut rest): (Vec<Expr>, Vec<Expr>) = (Vec::new(), Vec::new());
        for t in terms {
            if self.invariant(&t) && scalar(&t) {
                inv.push(t);
            } else {
                rest.push(t);
            }
        }
        // Nothing to gain unless the invariant terms form a real sub-sum.
        if inv.len() < 2 || rest.is_empty() {
            return;
        }
        let inv_sum = fold_add_terms(inv);
        let mut combined = rest;
        combined.push(inv_sum);
        *e = fold_add_terms(combined);
    }

    fn expr(&mut self, e: &mut Expr) {
        // Group the invariant terms of a sum before hoisting. `acc + inv_a +
        // inv_b` parses left-associatively, so no sub-expression is invariant
        // and the pass would otherwise hoist each term separately, leaving the
        // loop with one dependent add per term. Regrouping to `acc + (inv_a +
        // inv_b)` makes the invariant half a single hoistable value, so the loop
        // does one add per iteration. This is the reassociation a backend such
        // as LLVM performs, and is what makes a hot accumulation loop compile to
        // one add per iteration instead of one per invariant operand.
        self.regroup_sum(e);
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
                    mutable: false,
                });
                self.seen.insert(key, (slot, ty.clone()));
                *e = Expr::Var { name: format!("\u{0}licm{slot}"), slot, ty };
            }
            return;
        }
        match e {
Expr::PatScrutinee(_) => {
                unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
            }
            Expr::PatCheck { .. } => unreachable!("sema rewrites PatCheck into `true`"),
            Expr::PatArity { .. } => unreachable!("sema rewrites PatArity into `true`"),
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
            Expr::Block(b) | Expr::Loop(b, _, _, _) => self.block(b),
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
            Expr::TupleLit(v) => {
                for x in v {
                    self.expr(x);
                }
            }
            Expr::VariantTest { scrutinee, .. } => self.expr(scrutinee),
            Expr::BoolTest { scrutinee, .. } => self.expr(scrutinee),
            Expr::EnumPayload { recv, .. } => self.expr(recv),
            Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => {}
        }
    }

    /// True when a field read's base is a frame access rather than a load
    /// through a pointer that could fault. A local aggregate is a frame access.
    /// A local reference is a frame access only when it is *proven* to point at
    /// a live local (`self.proven`); an unproven reference, a reference-typed
    /// field, or any deeper chain holds an address that may be null or dangle.
    fn frame_field_base(&self, e: &Expr) -> bool {
        match e {
            Expr::Var { slot, ty, .. } => match ty {
                Type::Ref(..) => self.proven.contains(slot),
                _ => true,
            },
            Expr::Field { base, ty, .. } => {
                !matches!(ty, Type::Ref(..)) && self.frame_field_base(base)
            }
            _ => false,
        }
    }

    /// True when `e` reads only loop-invariant data and cannot trap or write.
    fn invariant(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Bool(_) | Expr::EnumVariant { .. } => true,
            Expr::Var { slot, .. } => !self.eff.all && !self.eff.slots.contains(slot),
            // A field read is memory, so it is only invariant when the loop
            // writes no memory at all.
            // A field read through a local aggregate is a frame access and
            // cannot fault, so it is safe to speculate. A read through a
            // reference may dereference a pointer that is null or dangling, so
            // it is hoisted only when the reference is proven to point at a live
            // local (`frame_field_base`): hoisting an unproven one would fault
            // on a loop whose body never runs.
            Expr::Field { base, ty, .. } => {
                !self.eff.mem
                    && ty.is_scalar()
                    && self.invariant(base)
                    && self.frame_field_base(base)
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

/// Flatten a `+` chain into its operands, left to right. A non-`Add` node is a
/// single term. Only `+` is flattened, so a nested `*` stays one operand.
fn collect_add_terms(e: &Expr) -> Vec<Expr> {
    match e {
        Expr::Bin(BinOp::Add, a, b) => {
            let mut v = collect_add_terms(a);
            v.extend(collect_add_terms(b));
            v
        }
        other => vec![other.clone()],
    }
}

/// Rebuild a `+` chain from `terms`, left to right, so it nests the same way the
/// parser would have produced it. Fewer than two terms yields the term itself.
fn fold_add_terms(mut terms: Vec<Expr>) -> Expr {
    let mut acc = terms.remove(0);
    for t in terms {
        acc = Expr::Bin(BinOp::Add, Box::new(acc), Box::new(t));
    }
    acc
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

/// The type of a hoistable expression: `Bool` for a comparison or a `bool`
/// `!`, the operand's type for a bitwise `!`, and `I64` otherwise. A `&`/`|`/`^`
/// on `bool` is `bool` (rustc's `BitAnd`/`BitOr`/`BitXor` impls), so a hoisted
/// one is bound to a `bool` slot; typing it `I64` would make a later `!` on the
/// hoisted `Var` a bitwise not.
fn expr_ty(e: &Expr) -> Type {
    match e {
        Expr::Bool(_) => Type::Bool,
        Expr::Var { ty, .. } | Expr::Field { ty, .. } => ty.clone(),
        Expr::Un(UnOp::Not, a) => expr_ty(a),
        Expr::Bin(op, _, _)
            if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) =>
        {
            Type::Bool
        }
        Expr::Bin(op, a, b) if matches!(op, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor) => {
            if expr_ty(a) == Type::Bool || expr_ty(b) == Type::Bool {
                Type::Bool
            } else {
                Type::I64
            }
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
        Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) | Expr::Var { .. } | Expr::EnumVariant { .. } => 1,
        Expr::Bin(_, a, b) => 1 + n(a) + n(b),
        Expr::Un(_, a) | Expr::Cast(a, _) => 1 + n(a),
        Expr::Field { base, .. } => 1 + n(base),
        _ => 1_000_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer, parser, sema};

    /// The proven-reference slots of `main` after a full front-end run.
    fn proven_main(src: &str) -> HashSet<i32> {
        let toks = lexer::lex(src).expect("lex");
        let mut prog = parser::Parser::new(toks).parse_program().expect("parse");
        sema::analyze(&mut prog).expect("sema");
        let main = prog.fns.iter().find(|f| f.name == "main").expect("main");
        proven_refs(&main.body)
    }

    /// The slot of the local named `name` in `main`.
    fn slot_of(src: &str, name: &str) -> i32 {
        let toks = lexer::lex(src).expect("lex");
        let mut prog = parser::Parser::new(toks).parse_program().expect("parse");
        sema::analyze(&mut prog).expect("sema");
        let main = prog.fns.iter().find(|f| f.name == "main").expect("main");
        find_slot(&main.body, name).expect("slot")
    }

    fn find_slot(b: &Block, name: &str) -> Option<i32> {
        for s in &b.stmts {
            if let Stmt::Let { name: n, slot, .. } = s {
                if n == name {
                    return Some(*slot);
                }
            }
        }
        None
    }

    #[test]
    fn a_reference_to_a_local_is_proven() {
        // `let r = &v;` addresses a live local, so `r` may be speculated through.
        let src = "fn main() { let v: i64 = 1; let r: &i64 = &v; println!(\"{}\", *r); }";
        let r = slot_of(src, "r");
        assert!(proven_main(src).contains(&r));
    }

    #[test]
    fn a_reference_to_a_field_of_a_local_is_proven() {
        // `&s.inner` is `&local + offset`, still frame storage.
        let src = "struct I { x: i64 } struct S { inner: I } \
                   fn main() { let s: S = S { inner: I { x: 1 } }; \
                   let q: &I = &s.inner; println!(\"{}\", q.x); }";
        let q = slot_of(src, "q");
        assert!(proven_main(src).contains(&q));
    }

    #[test]
    fn a_reference_to_a_field_of_a_reference_is_not_proven() {
        // The reported bug: `&r.inner` where `r: &S` dereferences `r`, which may
        // be null, so `q` must not be treated as pointing at frame storage.
        let src = "struct I { x: i64 } struct S { inner: I } \
                   fn main() { let r: &S = 0 as &S; let q: &I = &r.inner; \
                   println!(\"{}\", q.x); }";
        let q = slot_of(src, "q");
        assert!(!proven_main(src).contains(&q));
    }

    #[test]
    fn a_reference_to_a_reference_local_itself_is_proven() {
        // `&r` (not `&r.field`) addresses the local pointer slot, which is live
        // frame storage, so it stays proven.
        let src = "fn main() { let v: i64 = 1; let r: &i64 = &v; \
                   let rr: &&i64 = &r; println!(\"{}\", **rr); }";
        let rr = slot_of(src, "rr");
        assert!(proven_main(src).contains(&rr));
    }

    #[test]
    fn a_deeper_reference_field_chain_is_not_proven() {
        // `&r.mid.leaf`: the base `r.mid` is reached through a reference, so no
        // link in the chain is frame storage.
        let src = "struct L { x: i64 } struct M { leaf: L } struct S { mid: M } \
                   fn main() { let r: &S = 0 as &S; let q: &L = &r.mid.leaf; \
                   println!(\"{}\", q.x); }";
        let q = slot_of(src, "q");
        assert!(!proven_main(src).contains(&q));
    }
}
