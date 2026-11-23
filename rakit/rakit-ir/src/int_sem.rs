//! Shared integer semantics for Rakit IR.
//!
//! Integer values are stored as zero-extended bit patterns of their type's
//! width (so `I8` values live in `0..=255`). Unsigned operations work on that
//! representation directly; signed operations sign-extend first. This keeps the
//! interpreter and the constant folder in agreement for narrow types.

use crate::ir::{BinOp, IntCc, Type};

pub fn width(ty: Type) -> Option<u32> {
    match ty {
        Type::I8 => Some(8),
        Type::I16 => Some(16),
        Type::I32 => Some(32),
        Type::I64 | Type::Ptr => Some(64),
        _ => None,
    }
}

pub fn mask(ty: Type) -> Option<u64> {
    width(ty).map(|bits| {
        if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        }
    })
}

/// Keep only the low `width(ty)` bits, zero-extended into `i64`.
pub fn wrap(ty: Type, v: i64) -> i64 {
    match mask(ty) {
        Some(m) => (v as u64 & m) as i64,
        None => v,
    }
}

/// Sign-extend the low `width(ty)` bits into `i64`.
pub fn sign_extend(ty: Type, v: i64) -> i64 {
    match width(ty) {
        Some(64) | None => v,
        Some(bits) => {
            let shift = 64 - bits;
            (v << shift) >> shift
        }
    }
}

pub fn shift_amount(ty: Type, n: i64) -> u32 {
    let bits = width(ty).unwrap_or(64);
    (n as u32) & (bits - 1)
}

/// Evaluate an integer binary op with the type's width and signedness.
/// Returns `None` for division by zero or a non-integer opcode.
pub fn eval_bin(op: BinOp, ty: Type, a: i64, b: i64) -> Option<i64> {
    if ty == Type::I128 {
        return None;
    }
    let x = wrap(ty, a);
    let y = wrap(ty, b);
    let r = match op {
        BinOp::Add => x.wrapping_add(y),
        BinOp::Sub => x.wrapping_sub(y),
        BinOp::Mul => x.wrapping_mul(y),
        BinOp::Sdiv => {
            let (sx, sy) = (sign_extend(ty, x), sign_extend(ty, y));
            if sy == 0 {
                return None;
            }
            sx.wrapping_div(sy)
        }
        BinOp::Udiv => {
            if y == 0 {
                return None;
            }
            (x as u64).wrapping_div(y as u64) as i64
        }
        BinOp::Srem => {
            let (sx, sy) = (sign_extend(ty, x), sign_extend(ty, y));
            if sy == 0 {
                return None;
            }
            sx.wrapping_rem(sy)
        }
        BinOp::Urem => {
            if y == 0 {
                return None;
            }
            (x as u64).wrapping_rem(y as u64) as i64
        }
        BinOp::And => x & y,
        BinOp::Or => x | y,
        BinOp::Xor => x ^ y,
        BinOp::Shl => x.wrapping_shl(shift_amount(ty, y)),
        BinOp::Shr => sign_extend(ty, x).wrapping_shr(shift_amount(ty, y)),
        BinOp::Ushr => (x as u64).wrapping_shr(shift_amount(ty, y)) as i64,
        _ => return None,
    };
    Some(wrap(ty, r))
}

pub fn eval_icmp(cc: IntCc, ty: Type, a: i64, b: i64) -> bool {
    let x = wrap(ty, a);
    let y = wrap(ty, b);
    let (sx, sy) = (sign_extend(ty, x), sign_extend(ty, y));
    match cc {
        IntCc::Eq => x == y,
        IntCc::Ne => x != y,
        IntCc::Slt => sx < sy,
        IntCc::Sle => sx <= sy,
        IntCc::Sgt => sx > sy,
        IntCc::Sge => sx >= sy,
        IntCc::Ult => (x as u64) < (y as u64),
        IntCc::Ule => (x as u64) <= (y as u64),
        IntCc::Ugt => (x as u64) > (y as u64),
        IntCc::Uge => (x as u64) >= (y as u64),
    }
}
