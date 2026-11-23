//! RIR port of the divisibility-test fold.
//!
//! For odd `C`, with `m = C^-1 mod 2^64` and `T = (2^64 - 1) / C`:
//!
//! ```text
//! x % C == 0   <=>   (x * m mod 2^64) <= T
//! ```
//!
//! This module rewrites `icmp eq/ne (urem x, C), 0` for odd `C > 1` on `I64`.
//! Even divisors, narrower integer types and `brif`-on-remainder are still
//! handled by the CLIF pass: RIR's `brif` takes a `Bool`, so the remainder
//! always reaches it through an `icmp`, and the CLIF pass keeps covering the
//! rest of the instruction set.

use std::collections::HashMap;

use crate::int_sem;
use crate::ir::{BinOp, Function, Inst, IntCc, Type, Value};

pub fn run(func: &mut Function) -> bool {
    let consts = collect_consts(func);
    let copies = collect_copies(func);
    let mut candidates = Vec::new();

    for (bi, blk) in func.blocks.iter().enumerate() {
        for (ii, inst) in blk.insts.iter().enumerate() {
            let Inst::Icmp(rr, cc, ty, a, b) = inst else { continue };
            if !matches!(*ty, Type::I8 | Type::I16 | Type::I32 | Type::I64)
                || !matches!(cc, IntCc::Eq | IntCc::Ne)
            {
                continue;
            }
            let rem = if resolve_u64(&consts, &copies, *b) == Some(0) {
                *a
            } else if resolve_u64(&consts, &copies, *a) == Some(0) {
                *b
            } else {
                continue;
            };
            let Some((x, divisor)) = find_urem(func, &consts, &copies, rem, *ty) else {
                continue;
            };
            if divisor <= 1 {
                continue;
            }
            candidates.push(Candidate { bi, ii, rr: *rr, cc: *cc, x, divisor, ty: *ty });
        }
    }

    if candidates.is_empty() {
        return false;
    }
    for cand in candidates {
        rewrite(func, &cand);
    }
    true
}

struct Candidate {
    bi: usize,
    ii: usize,
    rr: Value,
    cc: IntCc,
    x: Value,
    divisor: u64,
    ty: Type,
}

fn collect_consts(func: &Function) -> HashMap<Value, i64> {
    let mut map = HashMap::new();
    for blk in &func.blocks {
        for inst in &blk.insts {
            if let Inst::Iconst(r, _, v) = inst {
                map.insert(*r, *v);
            }
        }
    }
    map
}

fn collect_copies(func: &Function) -> HashMap<Value, Value> {
    let mut map = HashMap::new();
    for blk in &func.blocks {
        for inst in &blk.insts {
            if let Inst::Copy(r, _, src) = inst {
                map.insert(*r, *src);
            }
        }
    }
    map
}

fn resolve_u64(
    consts: &HashMap<Value, i64>,
    copies: &HashMap<Value, Value>,
    mut v: Value,
) -> Option<u64> {
    for _ in 0..64 {
        if let Some(c) = consts.get(&v) {
            return Some(*c as u64);
        }
        match copies.get(&v) {
            Some(next) => v = *next,
            None => return None,
        }
    }
    None
}

fn find_urem(
    func: &Function,
    consts: &HashMap<Value, i64>,
    copies: &HashMap<Value, Value>,
    mut v: Value,
    ty: Type,
) -> Option<(Value, u64)> {
    for _ in 0..64 {
        for blk in &func.blocks {
            for inst in &blk.insts {
                if let Inst::Bin(r, BinOp::Urem, bin_ty, x, c) = inst {
                    if *r == v && *bin_ty == ty {
                        return Some((*x, int_sem::wrap(ty, *consts.get(c)?) as u64));
                    }
                }
            }
        }
        match copies.get(&v) {
            Some(next) => v = *next,
            None => return None,
        }
    }
    None
}

fn new_value(func: &mut Function, ty: Type) -> Value {
    let v = Value(func.values.len() as u32);
    func.values.push(ty);
    v
}

fn rewrite(func: &mut Function, cand: &Candidate) {
    let ty = cand.ty;
    let bits = int_sem::width(ty).expect("candidate is an integer type");
    let full_mask = int_sem::mask(ty).expect("candidate is an integer type");
    let pow2 = cand.divisor.trailing_zeros();
    let odd = cand.divisor >> pow2;
    let mut prologue = Vec::new();
    let zero_v = new_value(func, ty);
    prologue.push(Inst::Iconst(zero_v, ty, 0));

    let mut is_zero = None;
    if pow2 > 0 {
        let mask = (1u64 << pow2) - 1;
        let mask_v = new_value(func, ty);
        let low = new_value(func, ty);
        let div = new_value(func, Type::Bool);
        prologue.push(Inst::Iconst(mask_v, ty, mask as i64));
        prologue.push(Inst::Bin(low, BinOp::And, ty, cand.x, mask_v));
        prologue.push(Inst::Icmp(div, IntCc::Eq, ty, low, zero_v));
        is_zero = Some(div);
    }
    if odd > 1 {
        let inv = mod_inverse(odd, bits);
        let threshold = full_mask / odd;
        let inv_v = new_value(func, ty);
        let prod = new_value(func, ty);
        let thr_v = new_value(func, ty);
        let odd_tmp = new_value(func, Type::Bool);
        prologue.push(Inst::Iconst(inv_v, ty, inv as i64));
        prologue.push(Inst::Bin(prod, BinOp::Mul, ty, cand.x, inv_v));
        prologue.push(Inst::Iconst(thr_v, ty, threshold as i64));
        prologue.push(Inst::Icmp(odd_tmp, IntCc::Ule, ty, prod, thr_v));
        is_zero = Some(match is_zero {
            Some(div) => {
                let both = new_value(func, Type::Bool);
                let false_v = new_value(func, Type::Bool);
                prologue.push(Inst::Iconst(false_v, Type::Bool, 0));
                prologue.push(Inst::Select(both, Type::Bool, div, odd_tmp, false_v));
                both
            }
            None => odd_tmp,
        });
    }

    let mut is_zero = is_zero.expect("divisor > 1 has a test");
    if cand.cc == IntCc::Ne {
        let neg = new_value(func, Type::Bool);
        let false_v = new_value(func, Type::Bool);
        let true_v = new_value(func, Type::Bool);
        prologue.push(Inst::Iconst(false_v, Type::Bool, 0));
        prologue.push(Inst::Iconst(true_v, Type::Bool, 1));
        prologue.push(Inst::Select(neg, Type::Bool, is_zero, false_v, true_v));
        is_zero = neg;
    }

    let n = prologue.len();
    let blk = &mut func.blocks[cand.bi];
    for (k, inst) in prologue.into_iter().enumerate() {
        blk.insts.insert(cand.ii + k, inst);
    }
    blk.insts[cand.ii + n] = Inst::Copy(cand.rr, Type::Bool, is_zero);
}

/// `C^-1 mod 2^bits` for odd `C`, by Newton iteration on the 2-adic inverse.
fn mod_inverse(odd: u64, bits: u32) -> u64 {
    debug_assert!(odd % 2 == 1);
    let mask = if bits == 64 { u64::MAX } else { (1u64 << bits) - 1 };
    let mut x = odd & mask;
    for _ in 0..6 {
        x = x.wrapping_mul(2u64.wrapping_sub(odd.wrapping_mul(x))) & mask;
    }
    debug_assert_eq!((odd.wrapping_mul(x)) & mask, 1);
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, IntCc, Type};
    use crate::{Builder, Val};

    fn build_mod_test(c: i64, ne: bool) -> Function {
        build_mod_test_ty(Type::I64, c, ne)
    }

    fn build_mod_test_ty(ty: Type, c: i64, ne: bool) -> Function {
        let mut b = Builder::new(vec![ty], vec![ty]);
        let e = b.func().entry;
        let x = b.func().block(e).params[0];
        let d = b.iconst(e, ty, c);
        let rem = b.bin(e, BinOp::Urem, ty, x, d);
        let zero = b.iconst(e, ty, 0);
        let cc = if ne { IntCc::Ne } else { IntCc::Eq };
        let cond = b.icmp(e, cc, ty, rem, zero);
        let t = b.add_block(vec![]);
        let f = b.add_block(vec![]);
        b.brif(e, cond, t, vec![], f, vec![]);
        let one = b.iconst(t, ty, 1);
        b.ret(t, vec![one]);
        let two = b.iconst(f, ty, 2);
        b.ret(f, vec![two]);
        b.into_func()
    }

    #[test]
    fn folds_odd_divisor_eq() {
        let mut f = build_mod_test(3, false);
        assert_eq!(crate::run(&f, &[Val::I(9)]).unwrap(), vec![Val::I(1)]);
        assert!(run(&mut f));
        assert!(crate::verify(&f).is_ok(), "{:?}", crate::verify(&f).err());
        assert_eq!(crate::run(&f, &[Val::I(9)]).unwrap(), vec![Val::I(1)]);
        assert_eq!(crate::run(&f, &[Val::I(10)]).unwrap(), vec![Val::I(2)]);
        crate::optimize(&mut f);
        assert!(
            !f.blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(i, Inst::Bin(_, BinOp::Urem, ..))))
        );
    }

    #[test]
    fn folds_odd_divisor_ne() {
        let mut f = build_mod_test(5, true);
        assert_eq!(crate::run(&f, &[Val::I(10)]).unwrap(), vec![Val::I(2)]);
        assert!(run(&mut f));
        assert!(crate::verify(&f).is_ok(), "{:?}", crate::verify(&f).err());
        assert_eq!(crate::run(&f, &[Val::I(10)]).unwrap(), vec![Val::I(2)]);
        assert_eq!(crate::run(&f, &[Val::I(11)]).unwrap(), vec![Val::I(1)]);
    }

    #[test]
    fn handles_even_and_power_of_two_divisors() {
        for (divisor, hit, miss) in [(4i64, 8u64, 9u64), (8, 16, 17), (12, 12, 18)] {
            let mut f = build_mod_test(divisor, false);
            assert_eq!(crate::run(&f, &[Val::I(hit as i64)]).unwrap(), vec![Val::I(1)]);
            assert!(run(&mut f), "divisor {divisor} not rewritten");
            assert!(crate::verify(&f).is_ok(), "{:?}", crate::verify(&f).err());
            assert_eq!(crate::run(&f, &[Val::I(hit as i64)]).unwrap(), vec![Val::I(1)]);
            assert_eq!(crate::run(&f, &[Val::I(miss as i64)]).unwrap(), vec![Val::I(2)]);
            crate::optimize(&mut f);
            assert!(
                !f.blocks
                    .iter()
                    .any(|b| b.insts.iter().any(|i| matches!(i, Inst::Bin(_, BinOp::Urem, ..))))
            );
        }

        let mut ne = build_mod_test(4, true);
        assert!(run(&mut ne));
        assert!(crate::verify(&ne).is_ok(), "{:?}", crate::verify(&ne).err());
        assert_eq!(crate::run(&ne, &[Val::I(8)]).unwrap(), vec![Val::I(2)]);
        assert_eq!(crate::run(&ne, &[Val::I(9)]).unwrap(), vec![Val::I(1)]);
    }

    #[test]
    fn inverse_is_correct_for_all_odd_divisors_up_to_4095() {
        for d in (1..4096u64).step_by(2) {
            let m = mod_inverse(d, 64);
            assert_eq!(d.wrapping_mul(m), 1, "inverse wrong for {d}");
        }
    }

    #[test]
    fn inverse_is_correct_for_narrow_widths() {
        for bits in [8u32, 16, 32] {
            let mask = (1u64 << bits) - 1;
            for d in (1..512u64).step_by(2) {
                let m = mod_inverse(d, bits);
                assert_eq!((d.wrapping_mul(m)) & mask, 1, "bits {bits} d {d}");
            }
        }
    }

    #[test]
    fn handles_narrow_types() {
        for (ty, divisor, hit, miss) in [
            (Type::I8, 3i64, 255u64, 254u64),
            (Type::I8, 4, 252, 253),
            (Type::I16, 3, 65535, 65534),
            (Type::I32, 3, 0xFFFF_FFFF, 0xFFFF_FFFE),
        ] {
            let mut f = build_mod_test_ty(ty, divisor, false);
            assert_eq!(
                crate::run(&f, &[Val::I(hit as i64)]).unwrap(),
                vec![Val::I(1)],
                "{ty:?} divisor {divisor} hit {hit}"
            );
            assert!(run(&mut f), "{ty:?} divisor {divisor} not rewritten");
            assert!(crate::verify(&f).is_ok(), "{:?}", crate::verify(&f).err());
            assert_eq!(crate::run(&f, &[Val::I(hit as i64)]).unwrap(), vec![Val::I(1)]);
            assert_eq!(crate::run(&f, &[Val::I(miss as i64)]).unwrap(), vec![Val::I(2)]);
            crate::optimize(&mut f);
            assert!(
                !f.blocks
                    .iter()
                    .any(|b| b.insts.iter().any(|i| matches!(i, Inst::Bin(_, BinOp::Urem, ..))))
            );
        }
    }
}
