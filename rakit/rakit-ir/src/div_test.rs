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

use crate::ir::{BinOp, Function, Inst, IntCc, Type, Value};

pub fn run(func: &mut Function) -> bool {
    let consts = collect_consts(func);
    let copies = collect_copies(func);
    let mut candidates = Vec::new();

    for (bi, blk) in func.blocks.iter().enumerate() {
        for (ii, inst) in blk.insts.iter().enumerate() {
            let Inst::Icmp(rr, cc, ty, a, b) = inst else { continue };
            if *ty != Type::I64 || !matches!(cc, IntCc::Eq | IntCc::Ne) {
                continue;
            }
            let rem = if resolve_u64(&consts, &copies, *b) == Some(0) {
                *a
            } else if resolve_u64(&consts, &copies, *a) == Some(0) {
                *b
            } else {
                continue;
            };
            let Some((x, divisor)) = find_urem(func, &consts, &copies, rem) else {
                continue;
            };
            if divisor <= 1 {
                continue;
            }
            candidates.push(Candidate { bi, ii, rr: *rr, cc: *cc, x, divisor });
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
) -> Option<(Value, u64)> {
    for _ in 0..64 {
        for blk in &func.blocks {
            for inst in &blk.insts {
                if let Inst::Bin(r, BinOp::Urem, Type::I64, x, c) = inst {
                    if *r == v {
                        return Some((*x, *consts.get(c)? as u64));
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
    let pow2 = cand.divisor.trailing_zeros();
    let odd = cand.divisor >> pow2;
    let mut prologue = Vec::new();
    let zero_v = new_value(func, Type::I64);
    prologue.push(Inst::Iconst(zero_v, Type::I64, 0));

    let mut is_zero = None;
    if pow2 > 0 {
        let mask = (1u64 << pow2) - 1;
        let mask_v = new_value(func, Type::I64);
        let low = new_value(func, Type::I64);
        let div = new_value(func, Type::Bool);
        prologue.push(Inst::Iconst(mask_v, Type::I64, mask as i64));
        prologue.push(Inst::Bin(low, BinOp::And, Type::I64, cand.x, mask_v));
        prologue.push(Inst::Icmp(div, IntCc::Eq, Type::I64, low, zero_v));
        is_zero = Some(div);
    }
    if odd > 1 {
        let inv = mod_inverse_u64(odd);
        let threshold = u64::MAX / odd;
        let inv_v = new_value(func, Type::I64);
        let prod = new_value(func, Type::I64);
        let thr_v = new_value(func, Type::I64);
        let odd_tmp = new_value(func, Type::Bool);
        prologue.push(Inst::Iconst(inv_v, Type::I64, inv as i64));
        prologue.push(Inst::Bin(prod, BinOp::Mul, Type::I64, cand.x, inv_v));
        prologue.push(Inst::Iconst(thr_v, Type::I64, threshold as i64));
        prologue.push(Inst::Icmp(odd_tmp, IntCc::Ule, Type::I64, prod, thr_v));
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

/// `C^-1 mod 2^64` for odd `C`, by Newton iteration on the 2-adic inverse.
fn mod_inverse_u64(odd: u64) -> u64 {
    debug_assert!(odd % 2 == 1);
    let mut x = odd;
    for _ in 0..6 {
        x = x.wrapping_mul(2u64.wrapping_sub(odd.wrapping_mul(x)));
    }
    debug_assert_eq!(odd.wrapping_mul(x), 1);
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, IntCc, Type};
    use crate::{Builder, Val};

    fn build_mod_test(c: i64, ne: bool) -> Function {
        let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
        let e = b.func().entry;
        let x = b.func().block(e).params[0];
        let d = b.iconst(e, Type::I64, c);
        let rem = b.bin(e, BinOp::Urem, Type::I64, x, d);
        let zero = b.iconst(e, Type::I64, 0);
        let cc = if ne { IntCc::Ne } else { IntCc::Eq };
        let cond = b.icmp(e, cc, Type::I64, rem, zero);
        let t = b.add_block(vec![]);
        let f = b.add_block(vec![]);
        b.brif(e, cond, t, vec![], f, vec![]);
        let one = b.iconst(t, Type::I64, 1);
        b.ret(t, vec![one]);
        let two = b.iconst(f, Type::I64, 2);
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
        assert!(!f
            .blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i, Inst::Bin(_, BinOp::Urem, ..)))));
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
            assert_eq!(
                crate::run(&f, &[Val::I(hit as i64)]).unwrap(),
                vec![Val::I(1)]
            );
            assert!(run(&mut f), "divisor {divisor} not rewritten");
            assert!(crate::verify(&f).is_ok(), "{:?}", crate::verify(&f).err());
            assert_eq!(
                crate::run(&f, &[Val::I(hit as i64)]).unwrap(),
                vec![Val::I(1)]
            );
            assert_eq!(
                crate::run(&f, &[Val::I(miss as i64)]).unwrap(),
                vec![Val::I(2)]
            );
            crate::optimize(&mut f);
            assert!(!f
                .blocks
                .iter()
                .any(|b| b.insts.iter().any(|i| matches!(i, Inst::Bin(_, BinOp::Urem, ..)))));
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
            let m = mod_inverse_u64(d);
            assert_eq!(d.wrapping_mul(m), 1, "inverse wrong for {d}");
        }
    }
}
