//! Turn a single self-recursive function into a loop, halving its call tree.
//!
//! The obvious `fib` is written as
//!
//! ```text
//! fn fib(n) { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); }
//! ```
//!
//! which makes *two* recursive calls per level. rustc folds the right spine —
//! the `fib(n - 2)` chain — into a loop so only one call per level remains, and
//! this pass does the same, in the one shape where the rewrite is exact:
//!
//! ```text
//! f(x) = f(g(x)) op f(h(x))        op is `+` or `*`
//! ```
//!
//! Because `op` is associative and commutative, unrolling the `h` chain gives
//!
//! ```text
//! f(x) = f(g(x)) op f(g(h(x))) op f(g(h(h(x)))) op ... op guard(h^k(x))
//! ```
//!
//! which is exactly the loop
//!
//! ```text
//! acc = identity                  // 0 for `+`, 1 for `*`
//! while !cond(x) { acc = acc op f(g(x)); x = h(x); }
//! return acc op guard(x)
//! ```
//!
//! This is not a tail-call rewrite: the recursive call `f(g(x))` stays a call.
//! It removes the *second* call per level, so the tree is halved — the same
//! constant-factor win rustc gets, and the reason `fib` is the one benchmark
//! rmc used to lose.
//!
//! The pass is deliberately narrow. The function must
//!
//! * take exactly one `i64` parameter and return `i64` (a free function, not a
//!   method) — `x` is the only state the transform has to track;
//! * have exactly two statements: a base-case `if cond { return guard; }` and a
//!   `return f(g) op f(h)` with both operands calls to itself;
//! * keep `cond`, `guard`, `g` and `h` free of calls and `return`s, so evaluating
//!   them a different number of times cannot change behaviour. rmc has no
//!   side-effecting user function, but a `print` nested in an argument would
//!   otherwise be duplicated by the loop, and a `return` nested in an argument
//!   would leave the transformed loop instead of the recursive frame that wrote
//!   it, dropping the accumulator.
//!
//! Reassociation is only exact for wrapping arithmetic, so the pass is skipped
//! entirely under `--overflow-checks`: the checked trap fires on the first
//! overflowing intermediate, and the loop adds terms in a different order than
//! the recursion, so it would trap on a sum that completes without overflow.

use crate::ast::*;

/// Rewrite every function that matches the right-spine shape. `overflow_checks`
/// disables the pass, because reassociating checked arithmetic is not exact.
pub fn run(prog: &mut Program, overflow_checks: bool) {
    if overflow_checks {
        return;
    }
    for f in &mut prog.fns {
        if let Some(body) = try_transform(f) {
            f.body = body;
            // The accumulator is a fresh frame slot past the function's own.
            f.nslots += 1;
        }
    }
}

fn try_transform(f: &FnDef) -> Option<Block> {
    if f.takes_self || f.params.len() != 1 {
        return None;
    }
    if f.params[0].1 != Type::I64 || f.ret != Type::I64 {
        return None;
    }
    let (pname, _) = &f.params[0];
    let pslot = f.param_slots[0];
    if f.body.tail.is_some() || f.body.stmts.len() != 2 {
        return None;
    }

    // Statement 0: `if cond { return guard; }`, no `else`.
    let (cond, guard) = match &f.body.stmts[0] {
        Stmt::Expr(Expr::If(c, then, None)) if then.tail.is_none() => match then.stmts.as_slice() {
            [Stmt::Return(Some(g))] => (c.as_ref().clone(), g.clone()),
            _ => return None,
        },
        _ => return None,
    };

    // Statement 1: `return f(g) op f(h)`, `op` associative.
    let (op, g, h) = match &f.body.stmts[1] {
        Stmt::Return(Some(Expr::Bin(op @ (BinOp::Add | BinOp::Mul), l, r))) => {
            let g = self_arg(l, &f.name)?;
            let h = self_arg(r, &f.name)?;
            (*op, g, h)
        }
        _ => return None,
    };

    // Every piece must be pure and must not leave the function early, or the
    // loop would duplicate a side effect or exit without adding the accumulator.
    if [&cond, &guard, &g, &h].iter().any(|e| has_call(e) || has_return(e)) {
        return None;
    }

    let acc_slot = f.nslots;
    let acc_name = format!("\u{0}tco{acc_slot}");
    let acc = Expr::Var { name: acc_name.clone(), slot: acc_slot, ty: Type::I64 };
    let x = Expr::Var { name: pname.clone(), slot: pslot, ty: Type::I64 };
    let identity = if op == BinOp::Add { Expr::Int(0) } else { Expr::Int(1) };

    let call_g = Expr::Call {
        name: f.name.clone(),
        args: vec![g],
        arg_slots: vec![(-1, 0)],
        ret_slot: -1,
        ret_ty: Type::I64,
    };
    let step = Stmt::Assign {
        target: acc.clone(),
        value: Expr::Bin(op, Box::new(acc.clone()), Box::new(call_g)),
    };
    let advance = Stmt::Assign { target: x.clone(), value: h };
    let loop_body = Block { stmts: vec![step, advance], tail: None };
    let not_cond = Expr::Un(UnOp::Not, Box::new(cond));
    let ret = Stmt::Return(Some(Expr::Bin(op, Box::new(acc.clone()), Box::new(guard))));

    Some(Block {
        stmts: vec![
            Stmt::Let {
                name: acc_name,
                ty: Some(Type::I64),
                init: Some(identity),
                slot: acc_slot,
                mutable: false,
            },
            Stmt::While(not_cond, loop_body),
            ret,
        ],
        tail: None,
    })
}

/// `f(arg)` for the function being transformed, yielding `arg`.
fn self_arg(e: &Expr, name: &str) -> Option<Expr> {
    match e {
        Expr::Call { name: n, args, .. } if n == name && args.len() == 1 => Some(args[0].clone()),
        _ => None,
    }
}

fn has_call(e: &Expr) -> bool {
    match e {
        Expr::Call { .. } => true,
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } => false,
        Expr::Bin(_, a, b) => has_call(a) || has_call(b),
        Expr::Un(_, a) => has_call(a),
        Expr::If(c, t, e) => {
            has_call(c) || t.stmts.iter().any(|s| stmt_has_call(s))
                || t.tail.as_deref().map_or(false, has_call)
                || e.as_ref().map_or(false, |b| {
                    b.stmts.iter().any(|s| stmt_has_call(s))
                        || b.tail.as_deref().map_or(false, has_call)
                })
        }
        Expr::Block(b) => {
            b.stmts.iter().any(|s| stmt_has_call(s)) || b.tail.as_deref().map_or(false, has_call)
        }
        Expr::MatchBoundary(b, _) => {
            b.stmts.iter().any(|s| stmt_has_call(s)) || b.tail.as_deref().map_or(false, has_call)
        }
        Expr::Index(a, b) => has_call(a) || has_call(b),
        Expr::Field { base, .. } => has_call(base),
        Expr::StructLit { fields, .. } => fields.iter().any(|fi| has_call(&fi.value)),
        Expr::ArrayLit(v, _) => has_call(v),
        Expr::ArrayList(v) => v.iter().any(has_call),
        Expr::MethodCall { recv, args, .. } => {
            has_call(recv) || args.iter().any(has_call)
        }
        Expr::EnumVariant { .. } => false,
        Expr::EnumCtor { args, .. } => args.iter().any(has_call),
        Expr::VariantTest { scrutinee, .. } => has_call(scrutinee),
        Expr::BoolTest { scrutinee, .. } => has_call(scrutinee),
        Expr::EnumPayload { recv, .. } => has_call(recv),
        Expr::Cast(a, _) => has_call(a),
        Expr::AddrOf { expr, .. } => has_call(expr),
        Expr::Deref { expr, .. } => has_call(expr),
    }
}

fn stmt_has_call(s: &Stmt) -> bool {
    match s {
        Stmt::Let { init, .. } => init.as_ref().map_or(false, has_call),
        Stmt::Assign { target, value } => has_call(target) || has_call(value),
        Stmt::Expr(e) => has_call(e),
        Stmt::While(c, b) => has_call(c) || b.stmts.iter().any(stmt_has_call),
        Stmt::Loop(b) => b.stmts.iter().any(stmt_has_call),
        Stmt::For { start, end, body, .. } => {
            has_call(start) || has_call(end) || body.stmts.iter().any(stmt_has_call)
        }
        Stmt::Return(v) => v.as_ref().map_or(false, has_call),
        Stmt::Break | Stmt::Continue => false,
    }
}

/// True when `e` contains a `return`, which would leave the function from inside
/// the transformed loop rather than from the recursive frame that wrote it.
fn has_return(e: &Expr) -> bool {
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } | Expr::Call { .. } => false,
        Expr::Bin(_, a, b) => has_return(a) || has_return(b),
        Expr::Un(_, a) | Expr::Cast(a, _) | Expr::AddrOf { expr: a, .. } | Expr::Deref { expr: a, .. } => {
            has_return(a)
        }
        Expr::If(c, t, e) => {
            has_return(c)
                || block_has_return(t)
                || e.as_ref().map_or(false, |b| block_has_return(b))
        }
        Expr::Block(b) => block_has_return(b),
        Expr::MatchBoundary(b, _) => block_has_return(b),
        Expr::Index(a, b) => has_return(a) || has_return(b),
        Expr::Field { base, .. } => has_return(base),
        Expr::StructLit { fields, .. } => fields.iter().any(|fi| has_return(&fi.value)),
        Expr::ArrayLit(v, _) => has_return(v),
        Expr::ArrayList(v) => v.iter().any(has_return),
        Expr::MethodCall { recv, args, .. } => has_return(recv) || args.iter().any(has_return),
        Expr::EnumVariant { .. } => false,
        Expr::EnumCtor { args, .. } => args.iter().any(has_return),
        Expr::VariantTest { scrutinee, .. } => has_return(scrutinee),
        Expr::BoolTest { scrutinee, .. } => has_return(scrutinee),
        Expr::EnumPayload { recv, .. } => has_return(recv),
    }
}

fn block_has_return(b: &Block) -> bool {
    b.stmts.iter().any(stmt_has_return) || b.tail.as_deref().map_or(false, has_return)
}

fn stmt_has_return(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) => true,
        Stmt::Let { init, .. } => init.as_ref().map_or(false, has_return),
        Stmt::Assign { target, value } => has_return(target) || has_return(value),
        Stmt::Expr(e) => has_return(e),
        Stmt::While(c, b) => has_return(c) || block_has_return(b),
        Stmt::Loop(b) => block_has_return(b),
        Stmt::For { start, end, body, .. } => {
            has_return(start) || has_return(end) || block_has_return(body)
        }
        Stmt::Break | Stmt::Continue => false,
    }
}
