//! Compile-time evaluation of constant integer expressions.
//!
//! Two callers share this: sema, to fold a constant index or operand the way
//! rustc's constant propagation does, and the parser, to evaluate a `const`
//! item's initializer so its value can be substituted at every use.

use crate::ast::{fits_int, to_unsigned, wrap_int, BinOp, Expr, Type, UnOp};
use std::collections::HashMap;

/// The value of a resolved constant: an `i64`, a `bool`, or a `char`. A narrow
/// integer constant also carries the width it was declared at, so a use is
/// substituted back at that width (`const N: u8 = 5` used in `N + 200` still
/// overflows a `u8`) rather than collapsing to a plain `i64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstVal {
    Int(i64),
    /// A narrow integer constant: its in-width value and the declared width.
    NarrowInt(i64, Type),
    Bool(bool),
    Char(u32),
}

/// Resolve a list of `const` items, each `(name, declared type, initializer)`,
/// into a name-to-value table. Forward references are allowed and cycles are
/// left unresolved, so an item that never resolves (a cycle, a call, a type
/// rmc does not fold) is simply absent from the result and the caller
/// diagnoses it. `i64` and `bool` items are folded; any other declared type is
/// skipped (the caller rejects it).
pub fn resolve(items: &[(String, Type, Expr)]) -> Result<HashMap<String, ConstVal>, String> {
    resolve_with(items, &HashMap::new(), &HashMap::new(), &HashMap::new())
}

/// Resolve `items` seeded with the values already in scope (`base_ints`,
/// `base_bools`, `base_chars`), so a block-scoped item may name an enclosing
/// constant. Each item's own name is removed from the base first: rustc resolves
/// a name in a `const` initializer to the item itself, so a self-reference is a
/// cycle, not a read of the outer constant of the same name. The result holds
/// only the items, never a base-only name.
pub fn resolve_with(
    items: &[(String, Type, Expr)],
    base_ints: &HashMap<String, i64>,
    base_bools: &HashMap<String, bool>,
    base_chars: &HashMap<String, u32>,
) -> Result<HashMap<String, ConstVal>, String> {
    let mut ints = base_ints.clone();
    let mut bools = base_bools.clone();
    let mut chars = base_chars.clone();
    // An item shadows an enclosing constant of the same name for the whole
    // block, so drop the base value: a reference inside any initializer
    // resolves to the item (a cycle when it is the item's own initializer),
    // never to the outer value.
    for (name, _, _) in items {
        ints.remove(name);
        bools.remove(name);
        chars.remove(name);
    }
    // Iterate to a fixpoint so a constant may name one declared later.
    loop {
        let mut progress = false;
        for (name, ty, value) in items {
            if ints.contains_key(name) || bools.contains_key(name) || chars.contains_key(name) {
                continue;
            }
            match ty {
                // A narrow integer item is folded at its declared width: the
                // initializer must fit (`const N: u8 = 300` is a literal out of
                // range, and `const N: u8 = 200 + 100` overflows a `u8`), and
                // the stored value is the in-width result.
                Type::Int { signed, bits } => {
                    match eval_int(value, &ints) {
                        Ok(v) if fits_int(v, *signed, *bits) => {
                            ints.insert(name.clone(), wrap_int(v, *signed, *bits));
                            progress = true;
                        }
                        Ok(_) => {
                            return Err(format!(
                                "literal out of range for `{}`",
                                int_width_name(*signed, *bits)
                            ))
                        }
                        Err(e) => return Err(e),
                    }
                }
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
                Type::Char => {
                    if let Some(v) = eval_char(value, &chars) {
                        chars.insert(name.clone(), v);
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
    for (name, ty, _) in items {
        if let Some(v) = ints.get(name) {
            // A narrow item keeps its width in the value, so a use is
            // substituted back at that width rather than as a plain `i64`.
            if let Type::Int { .. } = ty {
                out.insert(name.clone(), ConstVal::NarrowInt(*v, ty.clone()));
            } else {
                out.insert(name.clone(), ConstVal::Int(*v));
            }
        } else if let Some(v) = bools.get(name) {
            out.insert(name.clone(), ConstVal::Bool(*v));
        } else if let Some(v) = chars.get(name) {
            out.insert(name.clone(), ConstVal::Char(*v));
        }
    }
    Ok(out)
}

/// The Rust name of an integer width, for an out-of-range diagnostic.
fn int_width_name(signed: bool, bits: u16) -> String {
    if bits == 64 && signed {
        "i64".into()
    } else if bits == 64 {
        "u64".into()
    } else {
        format!("{}{bits}", if signed { "i" } else { "u" })
    }
}

/// Evaluate a constant integer expression to an unbounded `i64`, without the
/// width-relative overflow rejection `const_binop_checked` applies. Used to fold
/// a narrow `const` item's initializer, where the *declared width* decides
/// overflow: `const N: u8 = 200 + 100` folds to 300 and is then range-checked
/// against `u8`, rather than rejected as an `i64` overflow (it is not one).
/// Division by zero and an out-of-range shift are still errors. A `wrapping_mul`
/// wraps, as its name promises.
pub fn eval_int(e: &Expr, ints: &HashMap<String, i64>) -> Result<i64, String> {
    use BinOp::*;
    let not_const = || "not a constant integer expression".to_string();
    match e {
        Expr::Var { name, .. } => ints.get(name).copied().ok_or_else(not_const),
        Expr::Int(k) => Ok(*k),
        Expr::Un(UnOp::Neg, inner) => eval_int(inner, ints)?
            .checked_neg()
            .ok_or_else(|| "this arithmetic operation overflows".to_string()),
        Expr::Cast(inner, ty) if *ty == Type::I64 => eval_int(inner, ints),
        Expr::Cast(inner, Type::Int { signed, bits }) => {
            // A bare literal cast (`300 as u8`) is typed by the cast, so it must
            // fit the destination; a suffixed literal (`300u16 as u8`) carries
            // its own width and the cast narrows it. rustc rejects the former.
            if let Expr::Int(v) = inner.as_ref() {
                if *bits < 64 && !fits_int(*v, *signed, *bits) {
                    return Err(format!(
                        "literal out of range for `{}`",
                        int_width_name(*signed, *bits)
                    ));
                }
            }
            let v = eval_int(inner, ints)?;
            if *bits >= 64 {
                Ok(v)
            } else {
                Ok(wrap_int(v, *signed, *bits))
            }
        }
        Expr::Bin(op, a, b) => {
            let x = eval_int(a, ints)?;
            let y = eval_int(b, ints)?;
            match op {
                Add => Ok(x.wrapping_add(y)),
                Sub => Ok(x.wrapping_sub(y)),
                Mul | WrappingMul => Ok(x.wrapping_mul(y)),
                Div => {
                    if y == 0 {
                        return Err("division by zero panics at run time".into());
                    }
                    if x == i64::MIN && y == -1 {
                        return Err("attempt to divide with overflow panics at run time".into());
                    }
                    Ok(x / y)
                }
                Mod => {
                    if y == 0 {
                        return Err("division by zero panics at run time".into());
                    }
                    if x == i64::MIN && y == -1 {
                        return Err("attempt to divide with overflow panics at run time".into());
                    }
                    Ok(x % y)
                }
                Shl => {
                    if !(0..64).contains(&y) {
                        return Err("shift amount is out of range".into());
                    }
                    Ok(x << y)
                }
                Shr => {
                    if !(0..64).contains(&y) {
                        return Err("shift amount is out of range".into());
                    }
                    Ok(x >> y)
                }
                BitAnd => Ok(x & y),
                BitOr => Ok(x | y),
                BitXor => Ok(x ^ y),
                _ => Err(not_const()),
            }
        }
        _ => Err(not_const()),
    }
}

/// A constant `char` expression: a character literal, or another `char`
/// constant by name.
pub fn eval_char(e: &Expr, chars: &HashMap<String, u32>) -> Option<u32> {
    match e {
        Expr::Char(c) => Some(*c),
        Expr::Var { name, .. } => chars.get(name).copied(),
        _ => None,
    }
}

/// A constant `i64` expression, resolving a reference to another integer
/// constant by name (`ints`). This is the same folding as `const_i64` plus the
/// name lookup a `const` initializer needs.
pub fn eval_i64(e: &Expr, ints: &HashMap<String, i64>) -> Option<i64> {
    match e {
        Expr::Var { name, .. } => ints.get(name).copied(),
        Expr::Int(k) => Some(*k),
        Expr::Un(UnOp::Neg, inner) => eval_i64(inner, ints)?.checked_neg(),
        // A cast to `i64` (or any width, all of which rmc treats as `i64`) is
        // the identity here, so an array length written `N as usize` still
        // folds. A narrow cast (`N as u8`) folds to the width's value, matching
        // how a narrow constant use is substituted (`Cast(Int, u8)`).
        Expr::Cast(inner, ty) if *ty == Type::I64 => eval_i64(inner, ints),
        Expr::Cast(inner, Type::Int { signed, bits }) => {
            let v = eval_i64(inner, ints)?;
            if *bits >= 64 {
                Some(v)
            } else {
                Some(wrap_int(v, *signed, *bits))
            }
        }
        Expr::Bin(op, a, b) => {
            let x = eval_i64(a, ints)?;
            let y = eval_i64(b, ints)?;
            const_binop_checked(*op, x, y)
        }
        // `wrapping_mul` is `*` without the overflow trap, so rustc folds it in
        // a `const` item (unlike `*`, which is rejected there). The parser sees
        // the method-call form — the `WrappingMul` operator only appears after
        // sema lowers the method — so the fold is needed here too, or a
        // `const K: i64 = a.wrapping_mul(b)` is rejected as non-constant.
        Expr::MethodCall { recv, name, args } if name == "wrapping_mul" && args.len() == 1 => {
            let x = eval_i64(recv, ints)?;
            let y = eval_i64(&args[0], ints)?;
            Some(x.wrapping_mul(y))
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
            // `&`, `|` and `^` on `bool` are the logical operators (rustc's
            // `BitAnd`/`BitOr`/`BitXor` impls for `bool`), so a `const bool`
            // may be written with them.
            BitAnd => Some(eval_bool(a, ints, bools)? & eval_bool(b, ints, bools)?),
            BitOr => Some(eval_bool(a, ints, bools)? | eval_bool(b, ints, bools)?),
            BitXor => Some(eval_bool(a, ints, bools)? ^ eval_bool(b, ints, bools)?),
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
        // `usize` and the other integer widths are transparent aliases for
        // `i64`, so `i as usize` (the usual spelling of an index) is an
        // identity cast that must not hide a constant index from the bound
        // check. A *narrowing* cast (a suffixed literal `200u8`, or `300 as u8`)
        // folds to the width's value, exactly as sema emits it.
        Expr::Cast(inner, ty) if *ty == Type::I64 => const_i64(inner),
        Expr::Cast(inner, Type::Int { signed, bits }) => {
            let v = const_i64(inner)?;
            if *bits >= 64 {
                Some(v)
            } else {
                Some(wrap_int(v, *signed, *bits))
            }
        }
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
                // `wrapping_mul` never overflows, so it folds to the wrapped
                // product rather than to `None`.
                WrappingMul => Some(x.wrapping_mul(y)),
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

/// Evaluate a constant binary operation at a width narrower than 64 bits the
/// way rustc does: `+`/`-`/`*` overflow, a division or remainder by zero, an
/// `MIN / -1`, and an out-of-range shift are rejected; `wrapping_mul` wraps.
/// The operands are in-width values (a two's-complement bit pattern in an
/// `i64`), so the result is the in-width result, matching what codegen computes.
pub fn int_const_binop(
    op: BinOp,
    x: i64,
    y: i64,
    signed: bool,
    bits: u16,
) -> Result<i64, String> {
    use BinOp::*;
    if bits >= 64 {
        return const_binop(op, x, y);
    }
    let overflow = || "this arithmetic operation overflows".to_string();
    match op {
        WrappingMul => Ok(wrap_int(x.wrapping_mul(y), signed, bits)),
        Add | Sub | Mul => {
            if signed {
                let a = wrap_int(x, true, bits) as i128;
                let b = wrap_int(y, true, bits) as i128;
                let r = match op {
                    Add => a + b,
                    Sub => a - b,
                    _ => a * b,
                };
                let lo = -(1i128 << (bits - 1));
                let hi = (1i128 << (bits - 1)) - 1;
                if r < lo || r > hi {
                    return Err(overflow());
                }
                Ok(wrap_int(r as i64, true, bits))
            } else {
                let a = to_unsigned(x, bits) as u128;
                let b = to_unsigned(y, bits) as u128;
                let r = match op {
                    Add => a + b,
                    Sub => a.wrapping_sub(b),
                    _ => a * b,
                };
                if r >= (1u128 << bits) {
                    return Err(overflow());
                }
                Ok(r as i64)
            }
        }
        Div | Mod => {
            if y == 0 {
                return Err("division by zero panics at run time".into());
            }
            if signed {
                let min = wrap_int(1i64 << (bits - 1), true, bits);
                if x == min && y == -1 {
                    return Err("attempt to divide with overflow panics at run time".into());
                }
                Ok(if op == Div { x / y } else { x % y })
            } else {
                let (a, b) = (to_unsigned(x, bits), to_unsigned(y, bits));
                Ok(if op == Div { (a / b) as i64 } else { (a % b) as i64 })
            }
        }
        Shl | Shr => {
            if !(0..bits as i64).contains(&y) {
                return Err(format!("shift amount is out of range for a {bits}-bit integer"));
            }
            Ok(if op == Shl {
                wrap_int(x.wrapping_shl(y as u32), signed, bits)
            } else if signed {
                x >> y
            } else {
                (to_unsigned(x, bits) >> y) as i64
            })
        }
        BitAnd => Ok(x & y),
        BitOr => Ok(x | y),
        BitXor => Ok(x ^ y),
        _ => Ok(0),
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
        // `wrapping_mul` is the one arithmetic op that cannot trap, so its
        // constant fold is always defined.
        WrappingMul => (a.wrapping_mul(b), false),
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
        WrappingMul => (a.wrapping_mul(b), false),
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
