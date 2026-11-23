//! Fold `x % C == 0` into a multiplication.
//!
//! `x % C` costs a hardware divide on x86-64 unless C is a power of two:
//! Rakit strength-reduces `urem` by a constant into a multiply-shift
//! sequence that still has to materialise the whole remainder. When the only
//! use of that remainder is a comparison against zero — the shape rustc emits
//! for `x % C == 0`, `x % C != 0` and `match x % C { 0 => .. }` — the remainder
//! itself is dead, and divisibility can be tested far more cheaply.
//!
//! For odd `C`, with `m = C^-1 mod 2^64` and `T = (2^64 - 1) / C`:
//!
//! ```text
//! x % C == 0   <=>   (x * m mod 2^64) <= T
//! ```
//!
//! because `m` is a unit, so `x * m mod 2^64` is injective, and the multiples
//! of `C` map onto exactly the `T + 1` values `0, m, 2m, .. T*m` that are
//! `<= T` — every value above `T` is `m * k` for a `k` that is not a multiple
//! of `C`. That is two instructions (`imul`, `cmp`) instead of the
//! multiply-shift remainder, and it is what LLVM emits. Measured on the
//! arithmetic benchmark, whose hot loop tests `acc % 7 == 0`, this is the whole
//! gap between rakit and LLVM.
//!
//! Even `C` is handled by peeling powers of two off first: divisibility by
//! `C = 2^s * d` is divisibility by `2^s` (a mask, which Rakit already
//! folds) and by odd `d` (the multiply above).
//!
//! `srem` is deliberately not rewritten: truncating division rounds toward
//! zero, so a negative `x` can have remainder zero when `x` is not a multiple
//! of `C`, and the identity above does not hold.

use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::condcodes::IntCC;
use rakit_codegen::ir::immediates::Imm64;
use rakit_codegen::ir::{self, Inst, InstBuilder, Opcode, Value, types};
use std::collections::HashMap as FxHashMap;

/// How the remainder's zero-ness is consumed.
enum UseSite {
    /// `icmp eq/ne rem, 0`; the payload says whether the result means "zero".
    Icmp { inst: Inst, is_equal: bool },
    /// `brif rem, ..` — true when the remainder is non-zero.
    Brif { inst: Inst },
}

/// A remainder whose only use is a test of whether it is zero.
struct Candidate {
    /// The `urem` instruction, the value it produces, and the divisor.
    rem_inst: Inst,
    rem_value: Value,
    divisor: u64,
    ty: types::Type,
    use_site: UseSite,
}

pub fn run(func: &mut ir::Function) {
    if !crate::tunables::div_test_enabled() {
        return;
    }
    let candidates = collect(func);
    for cand in candidates {
        rewrite(func, &cand);
    }
}

/// Find every `urem` by a constant whose sole use tests whether it is zero.
fn collect(func: &ir::Function) -> Vec<Candidate> {
    // One walk builds the use counts; a value used twice is not a candidate,
    // and the instruction that consumes it is recorded on the way.
    let mut uses: FxHashMap<Value, (usize, Option<Inst>)> = FxHashMap::default();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let data = &func.dfg.insts[inst];
            for &arg in data.arguments(&func.dfg.value_lists) {
                let arg = func.dfg.resolve_aliases(arg);
                let entry = uses.entry(arg).or_insert((0, None));
                entry.0 += 1;
                entry.1 = Some(inst);
            }
            // `arguments()` excludes branch (block-call) arguments, so a value
            // threaded into a successor's phi would otherwise look unused. Left
            // uncounted, removing the remainder's definition leaves that block
            // argument dangling and the egraph ICEs on it.
            for bc in data.branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables) {
                for arg in bc.args(&func.dfg.value_lists) {
                    if let ir::BlockArg::Value(v) = arg {
                        let v = func.dfg.resolve_aliases(v);
                        let entry = uses.entry(v).or_insert((0, None));
                        entry.0 += 1;
                        entry.1 = Some(inst);
                    }
                }
            }
            for &result in func.dfg.inst_results(inst) {
                uses.entry(result).or_insert((0, None));
            }
        }
    }

    let mut candidates = Vec::new();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let data = &func.dfg.insts[inst];
            if data.opcode() != Opcode::Urem {
                continue;
            }
            let args = data.arguments(&func.dfg.value_lists);
            let Some(divisor) = const_u64(&func.dfg, args[1]) else { continue };
            if divisor == 0 || divisor.is_power_of_two() {
                continue;
            }
            let results = func.dfg.inst_results(inst);
            if results.len() != 1 {
                continue;
            }
            let rem_value = results[0];
            let ty = func.dfg.value_type(rem_value);
            if !ty.is_int() || ty.is_vector() || ty.bits() > 64 {
                continue;
            }
            // Exactly one use, in the same block after the remainder, so
            // re-reading the dividend there still sees the same value.
            let Some(&(count, Some(user))) = uses.get(&rem_value) else { continue };
            if count != 1 {
                continue;
            }
            let Some(use_site) = classify_use(func, user, rem_value) else { continue };
            if !same_block_after(func, inst, user) {
                continue;
            }
            candidates.push(Candidate { rem_inst: inst, rem_value, divisor, ty, use_site });
        }
    }
    candidates
}

/// Decide whether `user` tests `rem_value` against zero, and how.
fn classify_use(func: &ir::Function, user: Inst, rem_value: Value) -> Option<UseSite> {
    let data = &func.dfg.insts[user];
    let args = data.arguments(&func.dfg.value_lists);
    match data.opcode() {
        // `switchInt(x)` lowers to a `brif` on `x != 0`.
        Opcode::Brif => {
            (func.dfg.resolve_aliases(args[0]) == rem_value).then_some(UseSite::Brif { inst: user })
        }
        Opcode::Icmp => {
            // Only `eq`/`ne` are accepted below, and both are symmetric, so
            // the operand order does not affect the condition.
            let arg0 = func.dfg.resolve_aliases(args[0]);
            let arg1 = func.dfg.resolve_aliases(args[1]);
            let (cond, other) = if arg0 == rem_value {
                (data.cond_code()?, arg1)
            } else if arg1 == rem_value {
                (data.cond_code()?, arg0)
            } else {
                return None;
            };
            if const_u64(&func.dfg, other) != Some(0) {
                return None;
            }
            let is_equal = match cond {
                IntCC::Equal => true,
                IntCC::NotEqual => false,
                _ => return None,
            };
            Some(UseSite::Icmp { inst: user, is_equal })
        }
        _ => None,
    }
}

/// Whether `b` follows `a` in the same block, so a value defined before `a`
/// still holds its value at `b`.
fn same_block_after(func: &ir::Function, a: Inst, b: Inst) -> bool {
    let Some(block) = func.layout.inst_block(a) else { return false };
    if func.layout.inst_block(b) != Some(block) {
        return false;
    }
    let mut seen = false;
    for inst in func.layout.block_insts(block) {
        if inst == a {
            seen = true;
        } else if inst == b {
            return seen;
        }
    }
    false
}

/// The value of a `u64` constant, if `value` is one.
fn const_u64(dfg: &ir::DataFlowGraph, value: Value) -> Option<u64> {
    let ir::ValueDef::Result(inst, 0) = dfg.value_def(value) else { return None };
    let ir::InstructionData::UnaryImm { imm, .. } = dfg.insts[inst] else { return None };
    Some(imm.bits() as u64)
}

/// `C^-1 mod 2^64` for odd `C`, by Newton iteration on the 2-adic inverse.
fn mod_inverse_u64(odd: u64) -> u64 {
    debug_assert!(odd % 2 == 1);
    // Seeding with `odd` is correct modulo 2^3, and each step doubles the
    // number of correct low bits, so five steps cover all 64.
    let mut x = odd;
    for _ in 0..5 {
        x = x.wrapping_mul(2u64.wrapping_sub(odd.wrapping_mul(x)));
    }
    debug_assert_eq!(odd.wrapping_mul(x), 1);
    x
}

/// Rewrite one candidate: test divisibility with a multiply, then drop the
/// now-dead remainder.
fn rewrite(func: &mut ir::Function, cand: &Candidate) {
    let ty = cand.ty;
    let pow2 = cand.divisor.trailing_zeros();
    let odd = cand.divisor >> pow2;
    let lhs = func.dfg.insts[cand.rem_inst].arguments(&func.dfg.value_lists)[0];
    let at = match cand.use_site {
        UseSite::Icmp { inst, .. } | UseSite::Brif { inst } => inst,
    };

    let mut pos = FuncCursor::new(func).at_inst(at);
    // Narrower-than-64-bit inputs are widened so the identity holds verbatim:
    // zero-extension preserves divisibility, and `urem` already interprets the
    // input as unsigned.
    let wide = if ty.bits() < 64 { pos.ins().uextend(types::I64, lhs) } else { lhs };
    // Peel the power-of-two factor: the low `pow2` bits must all be clear.
    let divisible = (pow2 > 0).then(|| {
        let mask = (1u64 << pow2) - 1;
        let low = pos.ins().band_imm_u(wide, Imm64::new(mask as i64));
        let zero = pos.ins().iconst(types::I64, 0);
        pos.ins().icmp(IntCC::Equal, low, zero)
    });
    let odd_test = (odd != 1).then(|| {
        let m = mod_inverse_u64(odd);
        let threshold = u64::MAX / odd;
        let prod = pos.ins().imul_imm_u(wide, Imm64::new(m as i64));
        let t = pos.ins().iconst(types::I64, threshold as i64);
        pos.ins().icmp(IntCC::UnsignedLessThanOrEqual, prod, t)
    });
    let is_zero = match (divisible, odd_test) {
        (Some(a), Some(b)) => pos.ins().band(a, b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => unreachable!("divisor is neither even nor odd"),
    };
    let replacement = match cand.use_site {
        UseSite::Icmp { is_equal: true, .. } => is_zero,
        // `brif` on `x != 0` can branch on `x == 0` instead, which is free;
        // only `icmp ne` needs the negation.
        UseSite::Brif { inst } => {
            let blocks = &mut pos.func.dfg.insts[inst];
            let ir::InstructionData::Brif { blocks: targets, .. } = blocks else {
                unreachable!("use site is a brif")
            };
            targets.swap(0, 1);
            is_zero
        }
        UseSite::Icmp { is_equal: false, .. } => {
            let one = pos.ins().iconst(types::I8, 1);
            pos.ins().bxor(is_zero, one)
        }
    };
    // The `brif` tests the remainder directly, so its operand is the condition.
    // An `icmp` is a separate value whose *result* other code consumes: that
    // result must be redirected, not the comparison's operands.
    match cand.use_site {
        UseSite::Brif { .. } => {
            let ir::DataFlowGraph { insts, value_lists, .. } = &mut pos.func.dfg;
            for arg in insts[at].arguments_mut(value_lists) {
                if *arg == cand.rem_value {
                    *arg = replacement;
                }
            }
        }
        UseSite::Icmp { .. } => {
            let result = pos.func.dfg.inst_results(at)[0];
            pos.func.dfg.detach_inst_results(at);
            pos.func.dfg.change_to_alias(result, replacement);
            pos.func.layout.remove_inst(at);
        }
    }
    pos.func.layout.remove_inst(cand.rem_inst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_is_correct_for_all_odd_divisors_up_to_4095() {
        for d in (1..4096u64).step_by(2) {
            let m = mod_inverse_u64(d);
            assert_eq!(d.wrapping_mul(m), 1, "inverse wrong for {d}");
        }
    }

    #[test]
    fn identity_holds_for_all_odd_divisors_up_to_511() {
        for d in (1..512u64).step_by(2) {
            let m = mod_inverse_u64(d);
            let t = u64::MAX / d;
            let mut vals: Vec<u64> = (0..2048).collect();
            vals.extend((0..1024).map(|k| d.wrapping_mul(k)));
            vals.extend((0..1024).map(|k| d.wrapping_mul(k).wrapping_add(1)));
            vals.extend([u64::MAX, u64::MAX - 1, u64::MAX - d, u64::MAX - d + 1]);
            for x in vals {
                let got = x.wrapping_mul(m) <= t;
                let want = x % d == 0;
                assert_eq!(got, want, "d={d} x={x}");
            }
        }
    }
}
