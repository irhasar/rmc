//! Vectorize one canonical map loop to a 4-lane `I32X4` body.
//!
//! This runs **after the egraph** (see `context.rs`), where the loop is in SSA
//! form with a block-parameter induction. The earlier `rakit-opt` pipeline runs
//! before the egraph, where the induction is still a stack slot, so this must
//! live in the backend.
//!
//! Target: header `H(i: i64)` brifs on a trip `i < n` (constant or a
//! loop-invariant runtime bound); single-block
//! body `B` computes `c[i] = (a[i] {^,+,-} b[i]) [* K]` for loop-invariant base
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
use crate::ir::immediates::Offset32;
use crate::ir::{
    self, Block, DataFlowGraph, Function, InstBuilder, Layout, MemFlagsData, Opcode, Value,
    ValueDef, types,
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

struct Plan {
    h: Block,
    preheader: Block,
    n: Bound,
    base_a: Value,
    base_b: Value,
    base_c: Value,
    op: Opcode,
    k: Option<i64>,
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
    // vector gate `i+4 <= n` would invert.
    if !(jumps_to(func, s0, h) && !jumps_to(func, s1, h)) {
        return None;
    }
    let b = s0;
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
            || !defined_outside(dfg, &func.layout, nv, h, b)
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
    if pargs.len() != 1 {
        return None;
    }
    let init = pargs[0].as_value()?;
    if iconst_i64(dfg, init)? != 0 {
        return None;
    }
    let bterm = func.layout.last_inst(b)?;
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
    for inst in func.layout.block_insts(b) {
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
    if loads.len() != 2 || stores.len() != 1 {
        return None;
    }
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
        if i32::from(offset) != 0 || dfg.value_type(dfg.first_result(inst)) != types::I32 {
            return None;
        }
        let addr = dfg.insts[inst].arguments(&dfg.value_lists).first().copied()?;
        let (base, hoff) = base_and_off(dfg, addr, i)?;
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
    if sargs.len() < 2 || dfg.value_type(dfg.resolve_aliases(sargs[0])) != types::I32 {
        return None;
    }
    let (store_base, coff) = base_and_off(dfg, sargs[1], i)?;
    if dfg.resolve_aliases(coff) != dfg.resolve_aliases(off?) {
        return None;
    }

    // Store value: `combine(loadA, loadB)`, optionally `* K` for a constant K.
    // Supported combines are the u32-lane ops xor / add / sub (all lower to
    // I32X4 instructions); other shapes stay scalar.
    let (top, tx, ty) = binary(dfg, sargs[0])?;
    let (op, lhs, rhs, k) = if top == Opcode::Imul {
        let (bx, kv) = if iconst_i64(dfg, ty).is_some() {
            (tx, ty)
        } else if iconst_i64(dfg, tx).is_some() {
            (ty, tx)
        } else {
            return None;
        };
        let k = iconst_i64(dfg, kv)?;
        let (xop, xa, xb) = binary(dfg, bx)?;
        (xop, xa, xb, Some(k))
    } else {
        (top, tx, ty, None)
    };
    if !matches!(op, Opcode::Bxor | Opcode::Iadd | Opcode::Isub) {
        return None;
    }
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

    for &base in &[base_lhs, base_rhs, store_base] {
        if !defined_outside(dfg, &func.layout, base, h, b) {
            return None;
        }
    }

    Some(Plan {
        h,
        preheader,
        n,
        base_a: dfg.resolve_aliases(base_lhs),
        base_b: dfg.resolve_aliases(base_rhs),
        base_c: dfg.resolve_aliases(store_base),
        op,
        k,
    })
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

    {
        let mut cur = FuncCursor::new(func).at_bottom(g);
        let zero = cur.ins().iconst(types::I64, 0);
        let four = cur.ins().iconst(types::I64, 4);
        let n = match p.n {
            Bound::Const(c) => cur.ins().iconst(types::I64, c),
            Bound::Runtime(v) => v,
        };
        let lbytes = cur.ins().imul(n, four);
        let (pa, pb, pc) = (p.base_a, p.base_b, p.base_c);
        let mut ov = None;
        for src in [pa, pb] {
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
        // `n*4` is only a faithful span when n < 2^62; for a larger runtime n the
        // multiply wraps, so fall back to the scalar loop (treat as overlapping).
        let lim = cur.ins().iconst(types::I64, 1i64 << 62);
        let big = cur.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, n, lim);
        let ov = cur.ins().bor(ov, big);
        cur.ins().brif(ov, h, &[zero.into()], vh, &[zero.into()]);
    }

    {
        let mut cur = FuncCursor::new(func).at_bottom(vh);
        let four = cur.ins().iconst(types::I64, 4);
        let nxt = cur.ins().iadd(vh_i, four);
        let n = match p.n {
            Bound::Const(c) => cur.ins().iconst(types::I64, c),
            Bound::Runtime(v) => v,
        };
        let cond = cur.ins().icmp(IntCC::UnsignedLessThanOrEqual, nxt, n);
        cur.ins().brif(cond, vb, &[vh_i.into()], h, &[vh_i.into()]);
    }

    {
        let mut cur = FuncCursor::new(func).at_bottom(vb);
        let two = cur.ins().iconst(types::I64, 2);
        let off = cur.ins().ishl(vb_i, two);
        let flags = MemFlagsData::new().with_notrap();
        let addr_a = cur.ins().iadd(p.base_a, off);
        let va = cur.ins().load(types::I32X4, flags, addr_a, Offset32::new(0));
        let addr_b = cur.ins().iadd(p.base_b, off);
        let vx = cur.ins().load(types::I32X4, flags, addr_b, Offset32::new(0));
        let combined = match p.op {
            Opcode::Bxor => cur.ins().bxor(va, vx),
            Opcode::Iadd => cur.ins().iadd(va, vx),
            Opcode::Isub => cur.ins().isub(va, vx),
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
        let four = cur.ins().iconst(types::I64, 4);
        let nxt = cur.ins().iadd(vb_i, four);
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

fn base_and_off(dfg: &DataFlowGraph, addr: Value, i: Value) -> Option<(Value, Value)> {
    let (op, x, y) = binary(dfg, addr)?;
    if op != Opcode::Iadd {
        return None;
    }
    for (base, off) in [(x, y), (y, x)] {
        if scaled_of(dfg, off, i).is_some() {
            return Some((base, off));
        }
    }
    None
}

fn scaled_of(dfg: &DataFlowGraph, v: Value, i: Value) -> Option<()> {
    let (op, a, b) = binary(dfg, v)?;
    match op {
        // `i << 2` only: the shifted value must be `i` and the amount 2, so a
        // reversed `2 << i` is rejected.
        Opcode::Ishl if dfg.resolve_aliases(a) == i && iconst_i64(dfg, b) == Some(2) => Some(()),
        // `i * 4` is commutative.
        Opcode::Imul
            if (dfg.resolve_aliases(a) == i && iconst_i64(dfg, b) == Some(4))
                || (dfg.resolve_aliases(b) == i && iconst_i64(dfg, a) == Some(4)) =>
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
