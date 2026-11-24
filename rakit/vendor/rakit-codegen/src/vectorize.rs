//! Vectorize one canonical map loop to a 4-lane `I32X4` body.
//!
//! This runs **after the egraph** (see `context.rs`), where the loop is in SSA
//! form with a block-parameter induction. The earlier `rakit-opt` pipeline runs
//! before the egraph, where the induction is still a stack slot, so this must
//! live in the backend.
//!
//! Target: header `H(i: i64)` brifs on a trip `i < n` (constant or a
//! loop-invariant runtime bound); single-block
//! body `B` computes `c[i] = (a[i] OP b[i]) [* K]`, `c[i] = a[i] OP K`, or
//! `c[i] = a[i] OP s` (loop-invariant scalar) for
//! loop-invariant base
//! pointers, `addr = base + (i << 2)` (u32 lanes), then `i += 1` and jumps `H`.
//!
//! The transform never edits `H`/`B`. It inserts a runtime disjointness guard
//! plus a vector loop:
//!   P -> G (guard) -> { H(0) [overlap: scalar] | VH(0) [disjoint] }
//!   VH(i): `i+4 <= n` ? VB(i) : H(i)          (original `H` is the scalar tail)
//!   VB(i): 16-byte load A/B, xor, `imul.i32x4` by `splat(K)`, 16-byte store C,
//!          `i += 4`, jump VH(i)
//!
//! The guard proves the stored buffer is disjoint from both loaded buffers over
//! the whole loop (`|dst - src| >= n*4` unsigned); distinct base pointers are
//! *not* enough (`dst = src + 1` overlaps). `H`/`B` are reused as the scalar
//! tail and overlap fallback, so there is no scalar-loop duplication and no SSA
//! value is deleted.

use alloc::vec::Vec;

use crate::cursor::{Cursor, FuncCursor};
use crate::ir::condcodes::IntCC;
use crate::ir::immediates::{Ieee32, Ieee64, Offset32};
use crate::ir::{
    self, Block, DataFlowGraph, Function, InstBuilder, Layout, MemFlagsData, Opcode, StackSlotData,
    StackSlotKind, Value, ValueDef, types,
};

/// Whether the pass is enabled at runtime (`RAKIT_NO_VECTORIZE` disables it).
pub fn enabled() -> bool {
    #[cfg(feature = "std")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var_os("RAKIT_NO_VECTORIZE").is_none())
    }
    #[cfg(not(feature = "std"))]
    {
        true
    }
}

/// Vectorize every canonical loop in `func`. Returns whether the CFG changed
/// (the caller must recompute cfg/domtree when it did).
///
/// We iterate a snapshot of the original blocks: `apply` only *adds* blocks and
/// keeps the scalar header/body (as the tail), so each original header is
/// matched at most once and the freshly created vector blocks are never
/// revisited. This matters because the inliner/unroller can duplicate the same
/// loop many times into one function (e.g. a constant-trip outer loop unrolled
/// 20x); vectorizing only the first copy leaves the rest scalar.
pub fn do_vectorize(func: &mut Function) -> bool {
    let headers: Vec<Block> = func.layout.blocks().collect();
    let mut changed = false;
    for h in headers {
        if !func.layout.is_block_inserted(h) {
            continue;
        }
        if let Some(plan) = match_header(func, h) {
            apply(func, plan);
            changed = true;
        } else if let Some(rp) = match_reduction(func, h) {
            apply_reduction(func, rp);
            changed = true;
        }
    }
    changed
}

/// Trip bound: a literal is rematerialized in the generated blocks (so it need
/// not be defined outside the loop — the egraph may rematerialize constants in
/// the header); a runtime value must be loop-invariant.
#[derive(Clone, Copy)]
enum Bound {
    Const(i64),
    Runtime(Value),
}

/// Element/lane kind. `per_vec` is the number of elements in a 128-bit vector.
#[derive(Clone, Copy, PartialEq)]
enum Lanes {
    U8,
    U16,
    U32,
    I64,
    F32,
    F64,
}

impl Lanes {
    fn from_scalar(t: ir::Type) -> Option<Lanes> {
        if t == types::I8 {
            Some(Lanes::U8)
        } else if t == types::I16 {
            Some(Lanes::U16)
        } else if t == types::I32 {
            Some(Lanes::U32)
        } else if t == types::I64 {
            Some(Lanes::I64)
        } else if t == types::F32 {
            Some(Lanes::F32)
        } else if t == types::F64 {
            Some(Lanes::F64)
        } else {
            None
        }
    }
    fn vec(self) -> ir::Type {
        match self {
            Lanes::U32 => types::I32X4,
            Lanes::I64 => types::I64X2,
            Lanes::F32 => types::F32X4,
            Lanes::F64 => types::F64X2,
            Lanes::U8 => types::I8X16,
            Lanes::U16 => types::I16X8,
        }
    }
    fn scalar(self) -> ir::Type {
        match self {
            Lanes::U32 => types::I32,
            Lanes::I64 => types::I64,
            Lanes::F32 => types::F32,
            Lanes::F64 => types::F64,
            Lanes::U8 => types::I8,
            Lanes::U16 => types::I16,
        }
    }
    fn shift(self) -> i64 {
        match self {
            Lanes::U8 => 0,
            Lanes::U16 => 1,
            Lanes::U32 | Lanes::F32 => 2,
            Lanes::I64 | Lanes::F64 => 3,
        }
    }
    fn size(self) -> u64 {
        match self {
            Lanes::U8 => 1,
            Lanes::U16 => 2,
            Lanes::U32 | Lanes::F32 => 4,
            Lanes::I64 | Lanes::F64 => 8,
        }
    }
    fn per_vec(self) -> i64 {
        match self {
            Lanes::U8 => 16,
            Lanes::U16 => 8,
            Lanes::U32 | Lanes::F32 => 4,
            Lanes::I64 | Lanes::F64 => 2,
        }
    }
}

/// Second operand of the vector body: a second stream (two-source) or a
/// broadcast constant (single-source).
#[derive(Clone, Copy)]
enum Rhs {
    Mem(Value),
    IntConst(i64),
    FConst { bits: u64, is_f32: bool },
    /// A loop-invariant scalar broadcast to every lane (`c[i] = a[i] OP s`).
    Scalar(Value),
}

struct Plan {
    h: Block,
    preheader: Block,
    n: Bound,
    base_a: Value,
    base_c: Value,
    init: Value,
    lanes: Lanes,
    op: Opcode,
    k: Option<i64>,
    rhs: Rhs,
}

fn match_header(func: &Function, h: Block) -> Option<Plan> {
    let dfg = &func.dfg;
    let params = dfg.block_params(h);
    if params.len() != 1 || dfg.value_type(params[0]) != types::I64 {
        return None;
    }
    let i = params[0];
    let term = func.layout.last_inst(h)?;
    let ir::InstructionData::Brif { arg: cond, blocks, .. } = dfg.insts[term] else {
        return None;
    };
    let (cc, cx, cy) = icmp_parts(dfg, cond)?;
    // Accept `i < n` (`ult i, n`) and the commuted `n > i` (`ugt n, i`): the
    // frontend picks either orientation depending on context (a standalone
    // dynamic loop lowers to `ugt`), and both mean "run while i < n".
    let n_side = match cc {
        IntCC::UnsignedLessThan if dfg.resolve_aliases(cx) == i => cy,
        IntCC::UnsignedGreaterThan if dfg.resolve_aliases(cy) == i => cx,
        _ => return None,
    };
    let s0 = blocks[0].block(&dfg.value_lists);
    let s1 = blocks[1].block(&dfg.value_lists);
    // The body must be the *true* edge of `i < n` (the loop runs while i < n).
    // A body on the false edge means the loop runs while `i >= n`, which the
    // vector gate would invert. The body may span several straight-line blocks
    // (`min`/`max` split the loads from the op+store), so follow the chain from
    // the true edge back to `h`.
    let chain = body_chain(func, s0, h)?;
    if body_chain(func, s1, h).is_some() {
        return None; // ambiguous: the false edge also returns to the header
    }
    let b = chain[0];
    let last = *chain.last().unwrap();
    // Trip bound: a literal is rematerialized in the guard, so it does not need
    // a dominating definition (the egraph rematerializes cheap constants at use
    // sites, which can put the constant inside the loop header — e.g. with
    // RAKIT_NO_LICM=1). A runtime bound must be loop-invariant (defined outside
    // H/B) and of type i64. The `i+4 <= n` gate and `|dst-src| >= n*4` guard
    // work for any runtime n; n == 0 falls through to the scalar tail.
    let nv = dfg.resolve_aliases(n_side);
    let n = if let Some(c) = iconst_i64(dfg, nv) {
        // n*4 must not overflow the span computation (see the guard's n<2^62 check).
        if c <= 0 || c >= (1i64 << 62) {
            return None;
        }
        Bound::Const(c)
    } else {
        if nv == i || dfg.value_type(nv) != types::I64
            || !defined_outside_chain(dfg, &func.layout, nv, h, &chain)
        {
            return None;
        }
        Bound::Runtime(nv)
    };
    // The scalar header runs once per iteration; the vector loop bypasses it for
    // the vectorized prefix, so it must be effect-free (pure arithmetic + the
    // comparison only).
    for inst in func.layout.block_insts(h) {
        if inst == term {
            continue;
        }
        let op = dfg.insts[inst].opcode();
        if op.is_call()
            || op.can_load()
            || op.can_store()
            || op.can_trap()
            || op.other_side_effects()
        {
            return None;
        }
    }
    if !dfg.block_params(b).is_empty() {
        return None;
    }
    let pb = preds(func, b);
    if pb.len() != 1 || pb[0] != h {
        return None;
    }
    let ph = preds(func, h);
    if ph.len() != 2 || !ph.contains(&last) {
        return None;
    }
    let preheader = *ph.iter().find(|x| **x != last)?;
    let pterm = func.layout.last_inst(preheader)?;
    let ir::InstructionData::Jump { destination, .. } = dfg.insts[pterm] else {
        return None;
    };
    if destination.block(&dfg.value_lists) != h {
        return None;
    }
    let pargs: Vec<_> = destination.args(&dfg.value_lists).collect();
    if pargs.len() != 1 {
        return None;
    }
    let init = pargs[0].as_value()?;
    let bterm = func.layout.last_inst(last)?;
    let ir::InstructionData::Jump { destination: bnext, .. } = dfg.insts[bterm] else {
        return None;
    };
    if bnext.block(&dfg.value_lists) != h {
        return None;
    }
    let bargs: Vec<_> = bnext.args(&dfg.value_lists).collect();
    if bargs.len() != 1 {
        return None;
    }
    let nextv = bargs[0].as_value()?;
    let (nop, na, nb) = binary(dfg, nextv)?;
    if nop != Opcode::Iadd
        || !((dfg.resolve_aliases(na) == i && iconst_i64(dfg, nb) == Some(1))
            || (dfg.resolve_aliases(nb) == i && iconst_i64(dfg, na) == Some(1)))
    {
        return None; // require exactly `i + 1` (reject strided / other updates)
    }

    let mut loads = Vec::new();
    let mut stores = Vec::new();
    for &blk in &chain {
        for inst in func.layout.block_insts(blk) {
            if inst == bterm {
                continue;
            }
            let op = dfg.insts[inst].opcode();
            if op.is_call() || op.other_side_effects() {
                return None;
            }
            if op.can_store() {
                stores.push(inst);
                continue;
            }
            if op.can_load() {
                loads.push(inst);
                continue;
            }
            if op.can_trap() {
                return None; // e.g. trapnz / division: the vector body would skip it
            }
        }
    }
    if loads.is_empty() || loads.len() > 2 || stores.len() != 1 {
        return None;
    }
    // Element type comes from the loads (both must agree) and fixes the lane
    // kind: u32 (xor/add/sub [*K]), i64 (xor/add/sub), f32 (add/sub/mul/div).
    let elem_ty = dfg.value_type(dfg.first_result(loads[0]));
    let lanes = Lanes::from_scalar(elem_ty)?;
    let (shift, size) = (lanes.shift(), lanes.size() as i64);
    let mut off: Option<Value> = None;
    let mut load_bases = Vec::new();
    for &inst in &loads {
        let ir::InstructionData::Load { offset, flags, .. } = dfg.insts[inst] else {
            return None;
        };
        if dfg.insts[inst].opcode() != Opcode::Load {
            return None; // reject uload8/sload8/… (narrow accesses)
        }
        let fl = &func.dfg.mem_flags[flags];
        if !fl.notrap() || fl.explicit_endianness().is_some() {
            return None; // would lose trap or byte-order semantics
        }
        if i32::from(offset) != 0 || dfg.value_type(dfg.first_result(inst)) != elem_ty {
            return None;
        }
        let addr = dfg.insts[inst].arguments(&dfg.value_lists).first().copied()?;
        let (base, hoff) = base_and_off(dfg, addr, i, shift, size)?;
        match off {
            Some(prev) if dfg.resolve_aliases(prev) != dfg.resolve_aliases(hoff) => return None,
            None => off = Some(hoff),
            _ => {}
        }
        load_bases.push(base);
    }
    let s = stores[0];
    let ir::InstructionData::Store { offset, flags, .. } = dfg.insts[s] else {
        return None;
    };
    if dfg.insts[s].opcode() != Opcode::Store {
        return None; // reject istore8/istore16 (narrow accesses)
    }
    let fl = &func.dfg.mem_flags[flags];
    if !fl.notrap() || fl.explicit_endianness().is_some() {
        return None;
    }
    if i32::from(offset) != 0 {
        return None;
    }
    let sargs = dfg.insts[s].arguments(&dfg.value_lists);
    if sargs.len() < 2 || dfg.value_type(dfg.resolve_aliases(sargs[0])) != elem_ty {
        return None;
    }
    let (store_base, coff) = base_and_off(dfg, sargs[1], i, shift, size)?;
    if dfg.resolve_aliases(coff) != dfg.resolve_aliases(off?) {
        return None;
    }

    // Two shapes:
    //   two sources: `combine(loadA, loadB)` [* K]   (K: u32 lanes only)
    //   one source : `combine(loadA, const)` (commutative ops accept `const` on
    //                either side; `-`/`/` require the load on the left).
    let (top, tx, ty) = binary(dfg, sargs[0])?;
    let (op, lhs, rhs, k) = if loads.len() == 2 && top == Opcode::Imul && lanes == Lanes::U32 {
        // `(a OP b) * K` where K is a constant, or a plain two-source
        // multiply `a * b` when neither operand is a constant.
        let const_side = if iconst_i64(dfg, ty).is_some() {
            Some((tx, ty))
        } else if iconst_i64(dfg, tx).is_some() {
            Some((ty, tx))
        } else {
            None
        };
        match const_side {
            Some((bx, kv)) => {
                let k = iconst_i64(dfg, kv)?;
                let (xop, xa, xb) = binary(dfg, bx)?;
                (xop, xa, xb, Some(k))
            }
            None => (Opcode::Imul, tx, ty, None),
        }
    } else {
        (top, tx, ty, None)
    };
    let ok = match lanes {
        Lanes::U32 => matches!(
            op,
            Opcode::Bxor
                | Opcode::Band
                | Opcode::Bor
                | Opcode::Iadd
                | Opcode::Isub
                | Opcode::Imul
                | Opcode::Ishl
                | Opcode::Ushr
                | Opcode::Sshr
        ),
        Lanes::I64 => matches!(
            op,
            Opcode::Bxor
                | Opcode::Band
                | Opcode::Bor
                | Opcode::Iadd
                | Opcode::Isub
                | Opcode::Ishl
                | Opcode::Ushr
        ),
        // u8: exact arith + unsigned min/max (pminub/pmaxub are SSE2; signed
        // byte min/max need SSE4.1, so smin/smax on I8 stay out).
        Lanes::U8 => matches!(
            op,
            Opcode::Bxor
                | Opcode::Band
                | Opcode::Bor
                | Opcode::Iadd
                | Opcode::Isub
                | Opcode::Umin
                | Opcode::Umax
        ),
        // u16: exact arith + signed min/max (pminsw/pmaxsw are SSE2; unsigned
        // word min/max need SSE4.1, so umin/umax on I16 stay out).
        Lanes::U16 => matches!(
            op,
            Opcode::Bxor
                | Opcode::Band
                | Opcode::Bor
                | Opcode::Iadd
                | Opcode::Isub
                | Opcode::Smin
                | Opcode::Smax
        ),
        Lanes::F32 | Lanes::F64 => {
            matches!(op, Opcode::Fadd | Opcode::Fsub | Opcode::Fmul | Opcode::Fdiv)
        }
    };
    if !ok {
        return None;
    }

    let (base_a, rhs_val) = if loads.len() == 2 {
        let (la, lb) = (dfg.resolve_aliases(lhs), dfg.resolve_aliases(rhs));
        let r0 = dfg.resolve_aliases(dfg.first_result(loads[0]));
        let r1 = dfg.resolve_aliases(dfg.first_result(loads[1]));
        // Keep operand order so `sub` (non-commutative) stays `lhs - rhs`.
        let (base_lhs, base_rhs) = if la == r0 && lb == r1 {
            (load_bases[0], load_bases[1])
        } else if la == r1 && lb == r0 {
            (load_bases[1], load_bases[0])
        } else {
            return None;
        };
        (base_lhs, Rhs::Mem(base_rhs))
    } else {
        let r0 = dfg.resolve_aliases(dfg.first_result(loads[0]));
        let (load_on_left, c) = if dfg.resolve_aliases(lhs) == r0 {
            (true, rhs)
        } else if dfg.resolve_aliases(rhs) == r0 {
            (false, lhs)
        } else {
            return None;
        };
        if !load_on_left && !is_commutative(op) {
            return None; // reject `K - a` / `K / a`
        }
        let cval = if let Some(ci) = iconst_i64(dfg, c) {
            Rhs::IntConst(ci)
        } else if let Some((bits, is_f32)) = fconst_bits(dfg, c) {
            Rhs::FConst { bits, is_f32 }
        } else if dfg.value_type(c) == elem_ty
            && defined_outside_chain(dfg, &func.layout, c, h, &chain)
        {
            Rhs::Scalar(c) // loop-invariant scalar broadcast
        } else {
            return None;
        };
        match (&cval, lanes) {
            (Rhs::IntConst(_), Lanes::U32 | Lanes::I64 | Lanes::U8 | Lanes::U16) => {}
            (Rhs::FConst { is_f32, .. }, Lanes::F32) if *is_f32 => {}
            (Rhs::FConst { is_f32, .. }, Lanes::F64) if !*is_f32 => {}
            (Rhs::Scalar(_), _) => {} // already checked == elem_ty
            _ => return None,
        }
        (load_bases[0], cval)
    };

    // Constant shifts only, with the amount in range (avoids masking differences
    // between scalar and packed shift semantics).
    if matches!(op, Opcode::Ishl | Opcode::Ushr | Opcode::Sshr) {
        let width = match lanes {
            Lanes::U32 => 32,
            Lanes::I64 => 64,
            _ => return None,
        };
        match rhs_val {
            Rhs::IntConst(c) if c >= 0 && c < width => {}
            // Uniform runtime amount: a loop-invariant scalar splatted to all
            // lanes; the x64 lowering masks it like the scalar path.
            Rhs::Scalar(_) => {}
            _ => return None,
        }
    }
    for &base in &[base_a, store_base] {
        if !defined_outside_chain(dfg, &func.layout, base, h, &chain) {
            return None;
        }
    }
    if let Rhs::Mem(bb) = rhs_val {
        if !defined_outside_chain(dfg, &func.layout, bb, h, &chain) {
            return None;
        }
    }

    Some(Plan {
        h,
        preheader,
        n,
        base_a: dfg.resolve_aliases(base_a),
        base_c: dfg.resolve_aliases(store_base),
        init,
        lanes,
        op,
        k,
        rhs: rhs_val,
    })
}

struct RPlan {
    h: Block,
    preheader: Block,
    n: Bound,
    base: Value,
    start: Value,
    init: Value,
    lanes: Lanes,
    op: Opcode,
}

/// Matches `acc = init; for i in 0..n { acc = acc + a[i] }` for integer lanes
/// (u32/i64). Float sums are excluded: the vector order would change rounding.
fn match_reduction(func: &Function, h: Block) -> Option<RPlan> {
    let dfg = &func.dfg;
    let hparams = dfg.block_params(h);
    if hparams.len() != 2 {
        return None;
    }
    let i = hparams[0];
    let acc = hparams[1];
    if dfg.value_type(i) != types::I64 {
        return None;
    }
    let elem_ty = dfg.value_type(acc);
    let lanes = Lanes::from_scalar(elem_ty)?;
    if !matches!(lanes, Lanes::U32 | Lanes::I64) {
        return None;
    }
    let (shift, size) = (lanes.shift(), lanes.size() as i64);

    let hterm = func.layout.last_inst(h)?;
    let ir::InstructionData::Brif { arg, blocks, .. } = dfg.insts[hterm] else {
        return None;
    };
    let cond = arg;
    let s0 = blocks[0].block(&dfg.value_lists);
    let s1 = blocks[1].block(&dfg.value_lists);
    if s0 == s1 {
        return None;
    }
    let b = s0;
    let (cc, cx, cy) = icmp_parts(dfg, cond)?;
    let n_side = match cc {
        IntCC::UnsignedLessThan if dfg.resolve_aliases(cx) == i => cy,
        IntCC::UnsignedGreaterThan if dfg.resolve_aliases(cy) == i => cx,
        _ => return None,
    };
    if dfg.value_type(n_side) != types::I64 {
        return None;
    }

    if !dfg.block_params(b).is_empty() {
        return None;
    }
    let pb = preds(func, b);
    if pb.len() != 1 || pb[0] != h {
        return None;
    }
    let ph = preds(func, h);
    if ph.len() != 2 || !ph.contains(&b) {
        return None;
    }
    let preheader = *ph.iter().find(|x| **x != b)?;

    let pterm = func.layout.last_inst(preheader)?;
    let ir::InstructionData::Jump { destination, .. } = dfg.insts[pterm] else {
        return None;
    };
    if destination.block(&dfg.value_lists) != h {
        return None;
    }
    let pargs: Vec<_> = destination.args(&dfg.value_lists).collect();
    if pargs.len() != 2 {
        return None;
    }
    let start = pargs[0].as_value()?;
    let init = pargs[1].as_value()?;

    let bterm = func.layout.last_inst(b)?;
    let ir::InstructionData::Jump { destination: bnext, .. } = dfg.insts[bterm] else {
        return None;
    };
    if bnext.block(&dfg.value_lists) != h {
        return None;
    }
    let bargs: Vec<_> = bnext.args(&dfg.value_lists).collect();
    if bargs.len() != 2 {
        return None;
    }
    let i2 = bargs[0].as_value()?;
    let acc2 = bargs[1].as_value()?;
    let (nop, na, nb) = binary(dfg, i2)?;
    if nop != Opcode::Iadd
        || !((dfg.resolve_aliases(na) == i && iconst_i64(dfg, nb) == Some(1))
            || (dfg.resolve_aliases(nb) == i && iconst_i64(dfg, na) == Some(1)))
    {
        return None;
    }
    let (aop, ax, ay) = binary(dfg, acc2)?;
    if aop != Opcode::Iadd {
        return None;
    }

    let mut loads = Vec::new();
    for inst in func.layout.block_insts(b) {
        if inst == bterm {
            continue;
        }
        let op = dfg.insts[inst].opcode();
        if op.is_call() || op.other_side_effects() || op.can_store() {
            return None;
        }
        if op.can_load() {
            loads.push(inst);
            continue;
        }
        if op.can_trap() {
            return None;
        }
    }
    if loads.len() != 1 {
        return None;
    }
    let l = loads[0];
    let ir::InstructionData::Load { offset, flags, .. } = dfg.insts[l] else {
        return None;
    };
    if dfg.insts[l].opcode() != Opcode::Load {
        return None;
    }
    let fl = &func.dfg.mem_flags[flags];
    if !fl.notrap() || fl.explicit_endianness().is_some() {
        return None;
    }
    if i32::from(offset) != 0 || dfg.value_type(dfg.first_result(l)) != elem_ty {
        return None;
    }
    let lres = dfg.resolve_aliases(dfg.first_result(l));
    let accr = dfg.resolve_aliases(acc);
    let (oa, ob) = (dfg.resolve_aliases(ax), dfg.resolve_aliases(ay));
    if !((oa == accr && ob == lres) || (oa == lres && ob == accr)) {
        return None;
    }

    let addr = dfg.insts[l].arguments(&dfg.value_lists).first().copied()?;
    let (base, _hoff) = base_and_off(dfg, addr, i, shift, size)?;
    if !defined_outside(dfg, &func.layout, base, h, b)
        || !defined_outside(dfg, &func.layout, n_side, h, b)
    {
        return None;
    }

    // `acc` and the load result must be used only by `acc2`, and `acc2` only as
    // the back-edge argument, so the vector body reproduces exactly this update.
    let acc2r = dfg.resolve_aliases(acc2);
    let (mut acc_uses, mut l_uses, mut acc2_uses) = (0, 0, 0);
    for inst in func.layout.block_insts(b) {
        if inst == bterm {
            continue;
        }
        for &a in dfg.insts[inst].arguments(&dfg.value_lists) {
            let r = dfg.resolve_aliases(a);
            if r == accr {
                acc_uses += 1;
            }
            if r == lres {
                l_uses += 1;
            }
            if r == acc2r {
                acc2_uses += 1;
            }
        }
    }
    if acc_uses != 1 || l_uses != 1 || acc2_uses != 0 {
        return None;
    }

    let n = match iconst_i64(dfg, n_side) {
        Some(c) if c > 0 && (c as u64) < u64::MAX / lanes.size() => Bound::Const(c),
        Some(_) => return None,
        None => Bound::Runtime(n_side),
    };

    Some(RPlan { h, preheader, n, base: dfg.resolve_aliases(base), start, init, lanes, op: Opcode::Iadd })
}

fn apply_reduction(func: &mut Function, p: RPlan) {
    let h = p.h;
    let vh = func.dfg.make_block();
    let vb = func.dfg.make_block();
    let red = func.dfg.make_block();
    func.layout.append_block(vh);
    func.layout.append_block(vb);
    func.layout.append_block(red);
    let vt = p.lanes.vec();
    let vh_i = func.dfg.append_block_param(vh, types::I64);
    let vh_a = func.dfg.append_block_param(vh, vt);
    let vb_i = func.dfg.append_block_param(vb, types::I64);
    let vb_a = func.dfg.append_block_param(vb, vt);
    let red_i = func.dfg.append_block_param(red, types::I64);
    let red_a = func.dfg.append_block_param(red, vt);

    let pterm = func.layout.last_inst(p.preheader).unwrap();
    func.layout.remove_inst(pterm);
    {
        let mut cur = FuncCursor::new(func).at_bottom(p.preheader);
        let zero = cur.ins().iconst(p.lanes.scalar(), 0);
        let zv = cur.ins().splat(vt, zero);
        cur.ins().jump(vh, &[p.start.into(), zv.into()]);
    }
    {
        let mut cur = FuncCursor::new(func).at_bottom(vh);
        let step = cur.ins().iconst(types::I64, p.lanes.per_vec());
        let nxt = cur.ins().iadd(vh_i, step);
        let n = match p.n {
            Bound::Const(c) => cur.ins().iconst(types::I64, c),
            Bound::Runtime(v) => v,
        };
        let cond = cur.ins().icmp(IntCC::UnsignedLessThanOrEqual, nxt, n);
        cur.ins().brif(cond, vb, &[vh_i.into(), vh_a.into()], red, &[vh_i.into(), vh_a.into()]);
    }
    {
        let mut cur = FuncCursor::new(func).at_bottom(vb);
        let shift = cur.ins().iconst(types::I64, p.lanes.shift());
        let off = cur.ins().ishl(vb_i, shift);
        let flags = MemFlagsData::new().with_notrap();
        let addr = cur.ins().iadd(p.base, off);
        let v = cur.ins().load(vt, flags, addr, Offset32::new(0));
        let acc2 = cur.ins().iadd(vb_a, v);
        let step = cur.ins().iconst(types::I64, p.lanes.per_vec());
        let nxt = cur.ins().iadd(vb_i, step);
        cur.ins().jump(vh, &[nxt.into(), acc2.into()]);
    }
    {
        let slot =
            func.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 4));
        let mut cur = FuncCursor::new(func).at_bottom(red);
        let flags = MemFlagsData::new().with_notrap();
        let addr = cur.ins().stack_addr(types::I64, slot, 0);
        cur.ins().store(flags, red_a, addr, Offset32::new(0));
        let scalar = p.lanes.scalar();
        let size = p.lanes.size() as i32;
        let mut sum = cur.ins().load(scalar, flags, addr, Offset32::new(0));
        for k in 1..p.lanes.per_vec() as i32 {
            let v = cur.ins().load(scalar, flags, addr, Offset32::new(k * size));
            sum = cur.ins().iadd(sum, v);
        }
        // Vector accumulator started at 0; add `init` once here so `n < per_vec`
        // (no vector iterations) yields exactly `init`.
        let total = cur.ins().iadd(p.init, sum);
        cur.ins().jump(h, &[red_i.into(), total.into()]);
    }
}

fn apply(func: &mut Function, p: Plan) {
    let h = p.h;
    let g = func.dfg.make_block();
    let vh = func.dfg.make_block();
    let vb = func.dfg.make_block();
    func.layout.append_block(g);
    func.layout.append_block(vh);
    func.layout.append_block(vb);
    let vh_i = func.dfg.append_block_param(vh, types::I64);
    let vb_i = func.dfg.append_block_param(vb, types::I64);

    let pterm = func.layout.last_inst(p.preheader).unwrap();
    func.layout.remove_inst(pterm);
    {
        let mut cur = FuncCursor::new(func).at_bottom(p.preheader);
        cur.ins().jump(g, &[]);
    }

    // An in-place update (`dst` base == a source base) touches the same address
    // for each index, so it needs no overlap guard.
    let pc_res = p.base_c;
    let mut srcs: Vec<Value> = Vec::new();
    if p.base_a != pc_res {
        srcs.push(p.base_a);
    }
    if let Rhs::Mem(bb) = p.rhs {
        if func.dfg.resolve_aliases(bb) != pc_res {
            srcs.push(bb);
        }
    }
    {
        let mut cur = FuncCursor::new(func).at_bottom(g);
        if srcs.is_empty() {
            // In-place: no other buffer can overlap this one.
            cur.ins().jump(vh, &[p.init.into()]);
        } else {
            let size = cur.ins().iconst(types::I64, p.lanes.size() as i64);
            let n = match p.n {
                Bound::Const(c) => cur.ins().iconst(types::I64, c),
                Bound::Runtime(v) => v,
            };
            let lbytes = cur.ins().imul(n, size);
            let pc = p.base_c;
            let mut ov = None;
            for src in srcs {
                let d1 = cur.ins().isub(pc, src);
                let o1 = cur.ins().icmp(IntCC::UnsignedLessThan, d1, lbytes);
                let d2 = cur.ins().isub(src, pc);
                let o2 = cur.ins().icmp(IntCC::UnsignedLessThan, d2, lbytes);
                let o = cur.ins().bor(o1, o2);
                ov = Some(match ov {
                    Some(prev) => cur.ins().bor(prev, o),
                    None => o,
                });
            }
            let ov = ov.unwrap();
            // `n*size` is only a faithful span when n < u64::MAX/size; for a
            // larger runtime n the multiply wraps, so fall back to scalar.
            let lim = cur.ins().iconst(types::I64, (u64::MAX / p.lanes.size()) as i64);
            let big = cur.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, n, lim);
            let ov = cur.ins().bor(ov, big);
            cur.ins().brif(ov, h, &[p.init.into()], vh, &[p.init.into()]);
        }
    }

    {
        let mut cur = FuncCursor::new(func).at_bottom(vh);
        let step = cur.ins().iconst(types::I64, p.lanes.per_vec());
        let nxt = cur.ins().iadd(vh_i, step);
        let n = match p.n {
            Bound::Const(c) => cur.ins().iconst(types::I64, c),
            Bound::Runtime(v) => v,
        };
        let cond = cur.ins().icmp(IntCC::UnsignedLessThanOrEqual, nxt, n);
        cur.ins().brif(cond, vb, &[vh_i.into()], h, &[vh_i.into()]);
    }

    {
        let mut cur = FuncCursor::new(func).at_bottom(vb);
        let shift = cur.ins().iconst(types::I64, p.lanes.shift());
        let off = cur.ins().ishl(vb_i, shift);
        let vt = p.lanes.vec();
        let flags = MemFlagsData::new().with_notrap();
        let addr_a = cur.ins().iadd(p.base_a, off);
        let va = cur.ins().load(vt, flags, addr_a, Offset32::new(0));
        let other = match p.rhs {
            Rhs::Mem(bb) => {
                let addr_b = cur.ins().iadd(bb, off);
                cur.ins().load(vt, flags, addr_b, Offset32::new(0))
            }
            Rhs::IntConst(c) => {
                let kc = cur.ins().iconst(p.lanes.scalar(), c);
                cur.ins().splat(vt, kc)
            }
            Rhs::FConst { bits, is_f32 } => {
                let fc = if is_f32 {
                    cur.ins().f32const(Ieee32::with_bits(bits as u32))
                } else {
                    cur.ins().f64const(Ieee64::with_bits(bits))
                };
                cur.ins().splat(vt, fc)
            }
            Rhs::Scalar(v) => cur.ins().splat(vt, v),
        };
        let combined = match p.op {
            Opcode::Bxor => cur.ins().bxor(va, other),
            Opcode::Band => cur.ins().band(va, other),
            Opcode::Bor => cur.ins().bor(va, other),
            Opcode::Iadd => cur.ins().iadd(va, other),
            Opcode::Isub => cur.ins().isub(va, other),
            Opcode::Ishl => cur.ins().ishl(va, other),
            Opcode::Ushr => cur.ins().ushr(va, other),
            Opcode::Sshr => cur.ins().sshr(va, other),
            Opcode::Imul => cur.ins().imul(va, other),
            Opcode::Umin => cur.ins().umin(va, other),
            Opcode::Umax => cur.ins().umax(va, other),
            Opcode::Smin => cur.ins().smin(va, other),
            Opcode::Smax => cur.ins().smax(va, other),
            Opcode::Fadd => cur.ins().fadd(va, other),
            Opcode::Fsub => cur.ins().fsub(va, other),
            Opcode::Fmul => cur.ins().fmul(va, other),
            Opcode::Fdiv => cur.ins().fdiv(va, other),
            _ => unreachable!(),
        };
        let val = match p.k {
            Some(k) => {
                let kc = cur.ins().iconst(types::I32, k);
                let kv = cur.ins().splat(types::I32X4, kc);
                cur.ins().imul(combined, kv)
            }
            None => combined,
        };
        let addr_c = cur.ins().iadd(p.base_c, off);
        cur.ins().store(flags, val, addr_c, Offset32::new(0));
        let step = cur.ins().iconst(types::I64, p.lanes.per_vec());
        let nxt = cur.ins().iadd(vb_i, step);
        cur.ins().jump(vh, &[nxt.into()]);
    }
}

fn binary(dfg: &DataFlowGraph, v: Value) -> Option<(Opcode, Value, Value)> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    let ir::InstructionData::Binary { opcode, args, .. } = dfg.insts[inst] else { return None };
    Some((opcode, args[0], args[1]))
}

fn icmp_parts(dfg: &DataFlowGraph, v: Value) -> Option<(IntCC, Value, Value)> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    let ir::InstructionData::IntCompare { opcode: Opcode::Icmp, cond, args, .. } = dfg.insts[inst]
    else {
        return None;
    };
    Some((cond, args[0], args[1]))
}

fn iconst_i64(dfg: &DataFlowGraph, v: Value) -> Option<i64> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    let ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. } = dfg.insts[inst] else {
        return None;
    };
    Some(imm.bits())
}

fn fconst_bits(dfg: &DataFlowGraph, v: Value) -> Option<(u64, bool)> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    match dfg.insts[inst] {
        ir::InstructionData::UnaryIeee32 { imm, .. } => Some((imm.bits() as u64, true)),
        ir::InstructionData::UnaryIeee64 { imm, .. } => Some((imm.bits(), false)),
        _ => None,
    }
}

fn is_commutative(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Bxor
            | Opcode::Band
            | Opcode::Bor
            | Opcode::Iadd
            | Opcode::Imul
            | Opcode::Fadd
            | Opcode::Fmul
            | Opcode::Umin
            | Opcode::Umax
            | Opcode::Smin
            | Opcode::Smax
    )
}

fn base_and_off(
    dfg: &DataFlowGraph,
    addr: Value,
    i: Value,
    shift: i64,
    size: i64,
) -> Option<(Value, Value)> {
    let (op, x, y) = binary(dfg, addr)?;
    if op != Opcode::Iadd {
        return None;
    }
    for (base, off) in [(x, y), (y, x)] {
        if scaled_of(dfg, off, i, shift, size).is_some() {
            return Some((base, off));
        }
    }
    None
}

fn scaled_of(dfg: &DataFlowGraph, v: Value, i: Value, shift: i64, size: i64) -> Option<()> {
    // `base + i` with no scaling (byte elements, shift 0).
    if shift == 0 && dfg.resolve_aliases(v) == i {
        return Some(());
    }
    let (op, a, b) = binary(dfg, v)?;
    match op {
        // `i << shift` only: the shifted value must be `i` (reject `shift << i`).
        Opcode::Ishl if dfg.resolve_aliases(a) == i && iconst_i64(dfg, b) == Some(shift) => {
            Some(())
        }
        // `i * size` is commutative.
        Opcode::Imul
            if (dfg.resolve_aliases(a) == i && iconst_i64(dfg, b) == Some(size))
                || (dfg.resolve_aliases(b) == i && iconst_i64(dfg, a) == Some(size)) =>
        {
            Some(())
        }
        // `i + i` == `i * 2` (u16 addressing).
        Opcode::Iadd
            if size == 2 && dfg.resolve_aliases(a) == i && dfg.resolve_aliases(b) == i =>
        {
            Some(())
        }
        _ => None,
    }
}

fn jumps_to(func: &Function, from: Block, target: Block) -> bool {
    let Some(t) = func.layout.last_inst(from) else { return false };
    if let ir::InstructionData::Jump { destination, .. } = func.dfg.insts[t] {
        destination.block(&func.dfg.value_lists) == target
    } else {
        false
    }
}

fn preds(func: &Function, target: Block) -> Vec<Block> {
    let mut out = Vec::new();
    for b in func.layout.blocks() {
        if let Some(t) = func.layout.last_inst(b) {
            for d in func.dfg.insts[t]
                .branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables)
            {
                if d.block(&func.dfg.value_lists) == target {
                    out.push(b);
                }
            }
        }
    }
    out
}

/// Follow a straight-line body chain from `start` back to `h`. Every block must
/// have exactly one predecessor (the previous chain block, or `h` for the first),
/// no block parameters, and end in an argument-less `jump`; the final block jumps
/// to `h`. Returns the chain (`start` .. last, excluding `h`), or None.
fn body_chain(func: &Function, start: Block, h: Block) -> Option<Vec<Block>> {
    let dfg = &func.dfg;
    let mut chain: Vec<Block> = Vec::new();
    let mut cur = start;
    let mut expect_pred = h;
    loop {
        if chain.len() > 16 || cur == h {
            return None;
        }
        if !dfg.block_params(cur).is_empty() {
            return None;
        }
        let p = preds(func, cur);
        if p.len() != 1 || p[0] != expect_pred {
            return None;
        }
        chain.push(cur);
        let t = func.layout.last_inst(cur)?;
        let ir::InstructionData::Jump { destination, .. } = dfg.insts[t] else {
            return None; // brif / return / call: not a straight-line body
        };
        let nxt = destination.block(&dfg.value_lists);
        if nxt == h {
            return Some(chain);
        }
        if destination.args(&dfg.value_lists).count() != 0 {
            return None;
        }
        expect_pred = cur;
        cur = nxt;
    }
}

fn defined_outside_chain(
    dfg: &DataFlowGraph,
    layout: &Layout,
    v: Value,
    h: Block,
    chain: &[Block],
) -> bool {
    let v = dfg.resolve_aliases(v);
    let inside = |blk: Block| blk == h || chain.contains(&blk);
    match dfg.value_def(v) {
        ValueDef::Result(inst, _) => match layout.inst_block(inst) {
            Some(blk) => !inside(blk),
            None => true,
        },
        ValueDef::Param(blk, _) => !inside(blk),
        _ => true,
    }
}

fn defined_outside(dfg: &DataFlowGraph, layout: &Layout, v: Value, h: Block, b: Block) -> bool {
    let v = dfg.resolve_aliases(v);
    match dfg.value_def(v) {
        ValueDef::Result(inst, _) => match layout.inst_block(inst) {
            Some(blk) => blk != h && blk != b,
            None => true,
        },
        ValueDef::Param(blk, _) => blk != h && blk != b,
        _ => true,
    }
}
