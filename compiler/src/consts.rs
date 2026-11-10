//! Compile-time evaluation of constant integer expressions.
//!
//! Two callers share this: sema, to fold a constant index or operand the way
//! rustc's constant propagation does, and the parser, to evaluate a `const`
//! item's initializer so its value can be substituted at every use.

use crate::ast::{BinOp, Expr, Type, UnOp};
use std::collections::HashMap;

/// The value of a resolved constant: an `i64` or a `bool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstVal {
    Int(i64),
    Bool(bool),
}

/// Resolve a list of `const` items, each `(name, declared type, initializer)`,
/// into a name-to-value table. Forward references are allowed and cycles are
/// left unresolved, so an item that never resolves (a cycle, a call, a type
/// rmc does not fold) is simply absent from the result and the caller
/// diagnoses it. `i64` and `bool` items are folded; any other declared type is
/// skipped (the caller rejects it).
pub fn resolve(items: &[(String, Type, Expr)]) -> HashMap<String, ConstVal> {
    let mut ints: HashMap<String, i64> = HashMap::new();
    let mut bools: HashMap<String, bool> = HashMap::new();
    // Iterate to a fixpoint so a constant may name one declared later.
    loop {
        let mut progress = false;
        for (name, ty, value) in items {
            if ints.contains_key(name) || bools.contains_key(name) {
                continue;
            }
            match ty {
                Type::I64 => {
                    if let Some(v) = eval_i64(value, &ints) {
                        ints.insert(name.clone(), v);
                        progress = true;
                    }
                }
                Type::Bool => {
                    if let Some(v) = eval_bool(value, &ints, &bools) {
                        bools.insert(name.clone(), v);
                        progress = true;
                    }
                }
                _ => {}
            }
        }
        if !progress {
            break;
        }
    }
    let mut out = HashMap::new();
    for (k, v) in ints {
        out.insert(k, ConstVal::Int(v));
    }
    for (k, v) in bools {
        out.insert(k, ConstVal::Bool(v));
    }
    out
}

/// A constant `i64` expression, resolving a reference to another integer
/// constant by name (`ints`). This is the same folding as `const_i64` plus the
/// name lookup a `const` initializer needs.
pub fn eval_i64(e: &Expr, ints: &HashMap<String, i64>) -> Option<i64> {
    match e {
        Expr::Var { name, .. } => ints.get(name).copied(),
        Expr::Int(k) => Some(*k),
        Expr::Un(UnOp::Neg, inner) => eval_i64(inner, ints)?.checked_neg(),
        Expr::Bin(op, a, b) => {
            let x = eval_i64(a, ints)?;
            let y = eval_i64(b, ints)?;
            const_binop_checked(*op, x, y)
        }
        _ => None,
    }
}

/// A constant `bool` expression, resolving references to integer and boolean
/// constants by name.
pub fn eval_bool(
    e: &Expr,
    ints: &HashMap<String, i64>,
    bools: &HashMap<String, bool>,
) -> Option<bool> {
    use BinOp::*;
    match e {
        Expr::Var { name, .. } => bools.get(name).copied(),
        Expr::Bool(b) => Some(*b),
        Expr::Un(UnOp::Not, inner) => Some(!eval_bool(inner, ints, bools)?),
        Expr::Bin(op, a, b) => match op {
            And => Some(eval_bool(a, ints, bools)? && eval_bool(b, ints, bools)?),
            Or => Some(eval_bool(a, ints, bools)? || eval_bool(b, ints, bools)?),
            Eq | Ne | Lt | Le | Gt | Ge => {
                if let (Some(x), Some(y)) = (eval_i64(a, ints), eval_i64(b, ints)) {
                    Some(match op {
                        Eq => x == y,
                        Ne => x != y,
                        Lt => x < y,
                        Le => x <= y,
                        Gt => x > y,
                        Ge => x >= y,
                        _ => unreachable!(),
                    })
                } else {
                    let (x, y) = (eval_bool(a, ints, bools)?, eval_bool(b, ints, bools)?);
                    Some(match op {
                        Eq => x == y,
                        Ne => x != y,
                        _ => return None,
                    })
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// The value of a constant integer expression, if it is one: an integer
/// literal, or a negated constant (`-1` parses as `Un(Neg, Int(1))`, so a
/// constant index like `a[-1]` is still recognised).
pub fn const_i64(e: &Expr) -> Option<i64> {
    match e {
        Expr::Int(k) => Some(*k),
        Expr::Un(UnOp::Neg, inner) => const_i64(inner)?.checked_neg(),
        // Fold the integer operators too, so a constant index written as an
        // expression (`a[1 + 4]`) is bounds-checked exactly like a literal.
        Expr::Bin(op, a, b) => {
            let x = const_i64(a)?;
            let y = const_i64(b)?;
            use BinOp::*;
            match op {
                Add => x.checked_add(y),
                Sub => x.checked_sub(y),
                Mul => x.checked_mul(y),
                Div => x.checked_div(y),
                Mod => x.checked_rem(y),
                BitAnd => Some(x & y),
                BitOr => Some(x | y),
                BitXor => Some(x ^ y),
                // A shift of 64 or more is out of range, so it is not a usable
                // constant here; the operator itself reports it via `const_binop`.
                Shl => (0..64).contains(&y).then(|| x.wrapping_shl(y as u32)),
                Shr => (0..64).contains(&y).then(|| x.wrapping_shr(y as u32)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Evaluate a constant `i64` binary operation the way rustc does at compile
/// time: division or remainder by zero and `MIN / -1` panic at runtime, and
/// `+`/`-`/`*` overflow and an out-of-range shift amount trap under the
/// overflow checks rmc emits. Rejecting these where both operands are literals
/// keeps rmc from compiling a program rustc refuses (and that would fault at
/// run time). Only literal operands are folded; rmc does not propagate
/// constants through locals, so `let d = 0; 1 / d` is not diagnosed here.
pub fn const_binop(op: BinOp, a: i64, b: i64) -> Result<i64, String> {
    use BinOp::*;
    let (val, overflow) = match op {
        Add => a.overflowing_add(b),
        Sub => a.overflowing_sub(b),
        Mul => a.overflowing_mul(b),
        Div | Mod => {
            if b == 0 {
                return Err("division by zero panics at run time".into());
            }
            if a == i64::MIN && b == -1 {
                return Err("attempt to divide with overflow panics at run time".into());
            }
            (if op == Div { a / b } else { a % b }, false)
        }
        Shl | Shr => {
            if !(0..64).contains(&b) {
                return Err("shift amount is out of range for `i64`".into());
            }
            (if op == Shl { a << b } else { a >> b }, false)
        }
        BitAnd => (a & b, false),
        BitOr => (a | b, false),
        BitXor => (a ^ b, false),
        _ => return Ok(0),
    };
    if overflow {
        return Err("this arithmetic operation overflows".into());
    }
    Ok(val)
}

/// The value of a constant binary operation, or `None` if it is not defined
/// (division by zero, `MIN / -1`, overflow, or an out-of-range shift). Used
/// both to diagnose a constant expression (`const_binop`) and to fold one while
/// evaluating a constant local (`const_of`), where an undefined result simply
/// means the value is not a usable constant.
pub fn const_binop_checked(op: BinOp, a: i64, b: i64) -> Option<i64> {
    use BinOp::*;
    let (val, overflow) = match op {
        Add => a.overflowing_add(b),
        Sub => a.overflowing_sub(b),
        Mul => a.overflowing_mul(b),
        Div | Mod => {
            if b == 0 || (a == i64::MIN && b == -1) {
                return None;
            }
            (if op == Div { a / b } else { a % b }, false)
        }
        Shl | Shr => {
            if !(0..64).contains(&b) {
                return None;
            }
            (if op == Shl { a << b } else { a >> b }, false)
        }
        BitAnd => (a & b, false),
        BitOr => (a | b, false),
        BitXor => (a ^ b, false),
        _ => return None,
    };
    if overflow {
        return None;
    }
    Some(val)
}
