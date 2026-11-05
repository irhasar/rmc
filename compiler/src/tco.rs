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
//! this pass does the same, in the shapes where the rewrite is exact:
//!
//! ```text
//! f(x) = t(x) op f(h(x))           op is `+` or `*`
//! ```
//!
//! Here `t` is any pure expression — it may itself contain a self-call, which
//! is what `tree(n) = 1 + tree(n - 1) + tree(n - 2)` needs: parsed left to
//! right that is `t op f(h)` with `t = 1 + f(n - 1)` and `h = n - 2`. When both
//! operands are self-calls (`fib`), `t` is just `f(g)` and this is the original
//! shape.
//!
//! Because `op` is associative, unrolling the `h` chain gives
//!
//! ```text
//! f(x) = t(x) op t(h(x)) op t(h(h(x))) op ... op guard(h^k(x))
//! ```
//!
//! which is exactly the loop
//!
//! ```text
//! acc = identity                  // 0 for `+`, 1 for `*`
//! while !cond(x) { acc = acc op t(x); x = h(x); }
//! return acc op guard(x)
//! ```
//!
//! This is not a tail-call rewrite: the recursive call inside `t` stays a call.
//! It removes the *second* call per level, so the tree is halved — the same
//! constant-factor win rustc gets, and the reason `fib` and `tree` are the
//! call-heavy benchmarks rmc used to lose.
//!
//! The pass is deliberately narrow. The function must
//!
//! * take exactly one `i64` parameter and return `i64` (a free function, not a
//!   method) — `x` is the only state the transform has to track;
//! * have exactly two statements: a base-case `if cond { return guard; }` and a
//!   `return t op f(h)` whose right operand is a call to itself;
//! * keep `cond`, `guard` and `h` free of calls and `return`s, and `t` free of
//!   `return`s and of calls to anything but itself, so evaluating them a
//!   different number of times cannot change behaviour. rmc has no
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

    // Statement 1: `return t op f(h)`, `op` associative, `f(h)` a self-call.
    let (op, t, h) = match &f.body.stmts[1] {
        Stmt::Return(Some(Expr::Bin(op @ (BinOp::Add | BinOp::Mul), l, r))) => {
            let h = self_arg(r, &f.name)?;
            (*op, l.as_ref().clone(), h)
        }
        _ => return None,
    };

    // `cond`, `guard` and `h` must be pure and must not leave the function
    // early, or the loop would duplicate a side effect or exit without adding
    // the accumulator. `t` is evaluated once per level in both forms, but it
    // may call only the function being transformed (so the loop cannot duplicate
    // a side effect of some *other* function).
    if [&cond, &guard, &h].iter().any(|e| has_call(e) || has_return(e))
        || has_return(&t)
        || calls_other(&t, &f.name)
    {
        return None;
    }

    let acc_slot = f.nslots;
    let acc_name = format!("\u{0}tco{acc_slot}");
    let acc = Expr::Var { name: acc_name.clone(), slot: acc_slot, ty: Type::I64 };
    let x = Expr::Var { name: pname.clone(), slot: pslot, ty: Type::I64 };
    let identity = if op == BinOp::Add { Expr::Int(0) } else { Expr::Int(1) };

    let step = Stmt::Assign {
        target: acc.clone(),
        value: Expr::Bin(op, Box::new(acc.clone()), Box::new(t)),
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
            Stmt::While(not_cond, loop_body, None),
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

/// True when `e` contains a call to a function other than `name`. The loop
/// evaluates `t` once per level, so a call inside it is fine only if it is the
/// self-call whose tree the rewrite is already halving; a call to any other
/// function could be duplicated or reordered relative to the recursion.
fn calls_other(e: &Expr, name: &str) -> bool {
    match e {
        Expr::Call { name: n, args, .. } => {
            n != name || args.iter().any(|a| calls_other(a, name))
        }
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } => false,
        Expr::PatScrutinee(_) => {
            unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
        }
        Expr::Bin(_, a, b) => calls_other(a, name) || calls_other(b, name),
        Expr::Un(_, a) | Expr::Cast(a, _) | Expr::AddrOf { expr: a, .. } | Expr::Deref { expr: a, .. } => {
            calls_other(a, name)
        }
        Expr::If(c, t, e) => {
            calls_other(c, name)
                || t.stmts.iter().any(|s| stmt_calls_other(s, name))
                || t.tail.as_deref().map_or(false, |x| calls_other(x, name))
                || e.as_ref().map_or(false, |b| {
                    b.stmts.iter().any(|s| stmt_calls_other(s, name))
                        || b.tail.as_deref().map_or(false, |x| calls_other(x, name))
                })
        }
        Expr::Block(b) | Expr::MatchBoundary(b, _) | Expr::Loop(b, _, _, _) => {
            b.stmts.iter().any(|s| stmt_calls_other(s, name))
                || b.tail.as_deref().map_or(false, |x| calls_other(x, name))
        }
        Expr::Index(a, b) => calls_other(a, name) || calls_other(b, name),
        Expr::Field { base, .. } => calls_other(base, name),
        Expr::StructLit { fields, .. } => fields.iter().any(|fi| calls_other(&fi.value, name)),
        Expr::ArrayLit(v, _) => calls_other(v, name),
        Expr::ArrayList(v) => v.iter().any(|x| calls_other(x, name)),
        Expr::MethodCall { recv, args, .. } => {
            calls_other(recv, name) || args.iter().any(|a| calls_other(a, name))
        }
        Expr::EnumVariant { .. } => false,
        Expr::EnumCtor { args, .. } => args.iter().any(|a| calls_other(a, name)),
        Expr::VariantTest { scrutinee, .. } => calls_other(scrutinee, name),
        Expr::BoolTest { scrutinee, .. } => calls_other(scrutinee, name),
        Expr::EnumPayload { recv, .. } => calls_other(recv, name),
    }
}

fn stmt_calls_other(s: &Stmt, name: &str) -> bool {
    match s {
        Stmt::Let { init, .. } => init.as_ref().map_or(false, |e| calls_other(e, name)),
        Stmt::Assign { target, value } => calls_other(target, name) || calls_other(value, name),
        Stmt::Expr(e) => calls_other(e, name),
        Stmt::While(c, b, _) => {
            calls_other(c, name) || b.stmts.iter().any(|s| stmt_calls_other(s, name))
        }
        Stmt::For { start, end, body, .. } => {
            calls_other(start, name)
                || calls_other(end, name)
                || body.stmts.iter().any(|s| stmt_calls_other(s, name))
        }
        Stmt::Return(v) => v.as_ref().map_or(false, |e| calls_other(e, name)),
        Stmt::Break(..) | Stmt::Continue(_) => false,
    }
}

fn has_call(e: &Expr) -> bool {
    match e {
        Expr::Call { .. } => true,
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } => false,
        Expr::PatScrutinee(_) => {
            unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
        }
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
        Expr::Block(b) | Expr::Loop(b, _, _, _) => {
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
        Stmt::While(c, b, _) => has_call(c) || b.stmts.iter().any(stmt_has_call),
        Stmt::For { start, end, body, .. } => {
            has_call(start) || has_call(end) || body.stmts.iter().any(stmt_has_call)
        }
        Stmt::Return(v) => v.as_ref().map_or(false, has_call),
        Stmt::Break(..) | Stmt::Continue(_) => false,
    }
}

/// True when `e` contains a `return`, which would leave the function from inside
/// the transformed loop rather than from the recursive frame that wrote it.
fn has_return(e: &Expr) -> bool {
    match e {
        Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) | Expr::Var { .. } => false,
        Expr::PatScrutinee(_) => {
            unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
        }
        // A `return` can hide in a call argument, e.g. `f({ if c { return 0; } x })`.
        Expr::Call { args, .. } => args.iter().any(has_return),
        Expr::Bin(_, a, b) => has_return(a) || has_return(b),
        Expr::Un(_, a) | Expr::Cast(a, _) | Expr::AddrOf { expr: a, .. } | Expr::Deref { expr: a, .. } => {
            has_return(a)
        }
        Expr::If(c, t, e) => {
            has_return(c)
                || block_has_return(t)
                || e.as_ref().map_or(false, |b| block_has_return(b))
        }
        Expr::Block(b) | Expr::Loop(b, _, _, _) => block_has_return(b),
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
        Stmt::While(c, b, _) => has_return(c) || block_has_return(b),
        Stmt::For { start, end, body, .. } => {
            has_return(start) || has_return(end) || block_has_return(body)
        }
        Stmt::Break(..) | Stmt::Continue(_) => false,
    }
}
