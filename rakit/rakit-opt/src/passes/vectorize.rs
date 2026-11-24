//! Vectorize one canonical map loop to a 4-lane `I32X4` body.
//!
//! Target (verified from CLIF): loop header `H(i)` brifs on a **constant** trip
//! `i < n`; single-block body `B` computes, for loop-invariant base pointers
//! `sa/sb/sc`,
//!   `c[i] = (a[i] ^ b[i]) * K`   with `addr = base + (i << 2)` (u32 lanes),
//! then `i += 1` and jumps back to `H`.
//!
//! The transform **never edits `H` or `B`**. It inserts a runtime
//! *disjointness guard* plus a new vector loop:
//!   preheader P -> G (guard) -> { H(0)  [overlap: scalar] | VH(0) [disjoint] }
//!   VH(i): `i+4 <= n` ? VB(i) : H(i)          (original `H` is the scalar tail)
//!   VB(i): 16-byte load A/B, xor, `imul.i32x4` by `splat(K)`, 16-byte store C,
//!          `i += 4`, jump VH(i)
//!
//! The guard proves the stored buffer is **disjoint** from both loaded buffers
//! over the whole loop (`|dst - src| >= n*4` unsigned); distinct base pointers
//! are *not* enough (`dst = src + 1` overlaps). Because `H`/`B` are reused as the
//! scalar tail and the overlap fallback, there is no scalar-loop duplication and
//! no SSA value is deleted.
use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::condcodes::IntCC;
use rakit_codegen::ir::immediates::Offset32;
use rakit_codegen::ir::{
    self, Block, InstBuilder, MemFlagsData, Opcode, Value, ValueDef, types,
};

pub fn run(func: &mut ir::Function) -> bool {
    if std::env::var_os("RAKIT_NO_VECTORIZE").is_some() {
        return false;
    }
    let Some(plan) = find_plan(func) else { return false };
    apply(func, plan);
    true
}

struct Plan {
    h: Block,
    preheader: Block,
    n: i64,
    base_a: Value,
    base_b: Value,
    base_c: Value,
    k: i64,
}

fn find_plan(func: &ir::Function) -> Option<Plan> {
    for h in func.layout.blocks().collect::<Vec<_>>() {
        if let Some(plan) = match_header(func, h) {
            return Some(plan);
        }
    }
    None
}

fn match_header(func: &ir::Function, h: Block) -> Option<Plan> {
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
    if cc != IntCC::UnsignedLessThan || dfg.resolve_aliases(cx) != i {
        return None;
    }
    let n = iconst_i64(dfg, cy)?;
    if n <= 0 {
        return None;
    }
    let s0 = blocks[0].block(&dfg.value_lists);
    let s1 = blocks[1].block(&dfg.value_lists);
    let (b, _e) = if jumps_to(func, s0, h) && !jumps_to(func, s1, h) {
        (s0, s1)
    } else if jumps_to(func, s1, h) && !jumps_to(func, s0, h) {
        (s1, s0)
    } else {
        return None;
    };
    if !dfg.block_params(b).is_empty() {
        return None;
    }
    let pb = preds(func, b);
    if pb.len() != 1 || pb[0] != h {
        return None;
    }
    // Preheader: `H` has exactly two predecessors, `B` and a distinct `P`.
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
        return None; // trip must start at 0 (matches the target shape)
    }
    // Latch: `B` jumps to `H(i + 1)`.
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
    if nop != Opcode::Iadd || dfg.resolve_aliases(if iconst_i64(dfg, nb) == Some(1) { na } else { nb }) != i {
        return None;
    }

    // Body memory ops: exactly two i32 loads and one i32 store, offset 0, all at
    // the same `base + scaled(i)` with `scaled(i) = i << 2` (or `i * 4`).
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
        } else if op.can_load() {
            loads.push(inst);
        }
    }
    if loads.len() != 2 || stores.len() != 1 {
        return None;
    }
    let mut off: Option<Value> = None;
    let mut load_bases = Vec::new();
    for &inst in &loads {
        let ir::InstructionData::Load { offset, .. } = dfg.insts[inst] else { return None };
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
    let ir::InstructionData::Store { offset, .. } = dfg.insts[s] else { return None };
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

    // Store value = `imul(bxor(load, load), K)`.
    let (mop, mx, my) = binary(dfg, sargs[0])?;
    if mop != Opcode::Imul {
        return None;
    }
    let (bx, kv) = if iconst_i64(dfg, my).is_some() {
        (mx, my)
    } else if iconst_i64(dfg, mx).is_some() {
        (my, mx)
    } else {
        return None;
    };
    let k = iconst_i64(dfg, kv)?;
    let (xop, xa, xb) = binary(dfg, bx)?;
    if xop != Opcode::Bxor {
        return None;
    }
    let (la, lb) = (dfg.resolve_aliases(xa), dfg.resolve_aliases(xb));
    let r0 = dfg.resolve_aliases(dfg.first_result(loads[0]));
    let r1 = dfg.resolve_aliases(dfg.first_result(loads[1]));
    if !((la == r0 && lb == r1) || (la == r1 && lb == r0)) {
        return None;
    }

    // All three bases must be loop-invariant (defined outside `H`/`B`).
    let scatter = [load_bases[0], load_bases[1], store_base];
    for &base in &scatter {
        if !defined_outside(dfg, &func.layout, base, h, b) {
            return None;
        }
    }

    Some(Plan {
        h,
        preheader,
        n,
        base_a: dfg.resolve_aliases(load_bases[0]),
        base_b: dfg.resolve_aliases(load_bases[1]),
        base_c: dfg.resolve_aliases(store_base),
        k,
    })
}

fn apply(func: &mut ir::Function, p: Plan) {
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

    // Guard: overlap iff `|dst - src| < n*4`; if either overlaps, stay scalar.
    {
        let mut cur = FuncCursor::new(func).at_bottom(g);
        let zero = cur.ins().iconst(types::I64, 0);
        let n = cur.ins().iconst(types::I64, p.n);
        let four = cur.ins().iconst(types::I64, 4);
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
        // overlap (true) -> scalar `H`; disjoint (false) -> vector `VH`.
        cur.ins().brif(ov, h, &[zero.into()], vh, &[zero.into()]);
    }

    // Vector header: `i + 4 <= n` ? VB(i) : H(i)  (H handles the scalar tail).
    {
        let mut cur = FuncCursor::new(func).at_bottom(vh);
        let four = cur.ins().iconst(types::I64, 4);
        let nxt = cur.ins().iadd(vh_i, four);
        let n = cur.ins().iconst(types::I64, p.n);
        let cond = cur.ins().icmp(IntCC::UnsignedLessThanOrEqual, nxt, n);
        cur.ins().brif(cond, vb, &[vh_i.into()], h, &[vh_i.into()]);
    }

    // Vector body: 4-wide 16-byte loads/stores at `base + (i << 2)`.
    {
        let mut cur = FuncCursor::new(func).at_bottom(vb);
        let two = cur.ins().iconst(types::I64, 2);
        let off = cur.ins().ishl(vb_i, two);
        let flags = MemFlagsData::new().with_notrap();
        let addr_a = cur.ins().iadd(p.base_a, off);
        let va = cur.ins().load(types::I32X4, flags, addr_a, Offset32::new(0));
        let addr_b = cur.ins().iadd(p.base_b, off);
        let vx = cur.ins().load(types::I32X4, flags, addr_b, Offset32::new(0));
        let xored = cur.ins().bxor(va, vx);
        let k = cur.ins().iconst(types::I32, p.k);
        let kv = cur.ins().splat(types::I32X4, k);
        let m = cur.ins().imul(xored, kv);
        let addr_c = cur.ins().iadd(p.base_c, off);
        cur.ins().store(flags, m, addr_c, Offset32::new(0));
        let four = cur.ins().iconst(types::I64, 4);
        let nxt = cur.ins().iadd(vb_i, four);
        cur.ins().jump(vh, &[nxt.into()]);
    }
}

// --- helpers -------------------------------------------------------------

fn binary(dfg: &ir::DataFlowGraph, v: Value) -> Option<(Opcode, Value, Value)> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    let ir::InstructionData::Binary { opcode, args, .. } = dfg.insts[inst] else { return None };
    Some((opcode, args[0], args[1]))
}

fn icmp_parts(dfg: &ir::DataFlowGraph, v: Value) -> Option<(IntCC, Value, Value)> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    let ir::InstructionData::IntCompare { opcode: Opcode::Icmp, cond, args, .. } = dfg.insts[inst]
    else {
        return None;
    };
    Some((cond, args[0], args[1]))
}

fn iconst_i64(dfg: &ir::DataFlowGraph, v: Value) -> Option<i64> {
    let v = dfg.resolve_aliases(v);
    let ValueDef::Result(inst, _) = dfg.value_def(v) else { return None };
    let ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. } = dfg.insts[inst] else {
        return None;
    };
    Some(imm.bits())
}

/// `addr = base + scaled(i)` where `scaled(i) = i << 2` or `i * 4`.
fn base_and_off(dfg: &ir::DataFlowGraph, addr: Value, i: Value) -> Option<(Value, Value)> {
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

fn scaled_of(dfg: &ir::DataFlowGraph, v: Value, i: Value) -> Option<()> {
    let (op, a, b) = binary(dfg, v)?;
    let (l, r) = (dfg.resolve_aliases(a), dfg.resolve_aliases(b));
    let (i_on_left, c) = if l == i { (true, b) } else if r == i { (false, a) } else { return None };
    let _ = i_on_left;
    match op {
        Opcode::Ishl if iconst_i64(dfg, c) == Some(2) => Some(()),
        Opcode::Imul if iconst_i64(dfg, c) == Some(4) => Some(()),
        _ => None,
    }
}

fn jumps_to(func: &ir::Function, from: Block, target: Block) -> bool {
    let Some(t) = func.layout.last_inst(from) else { return false };
    if let ir::InstructionData::Jump { destination, .. } = func.dfg.insts[t] {
        destination.block(&func.dfg.value_lists) == target
    } else {
        false
    }
}

fn preds(func: &ir::Function, target: Block) -> Vec<Block> {
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

fn defined_outside(
    dfg: &ir::DataFlowGraph,
    layout: &ir::Layout,
    v: Value,
    h: Block,
    b: Block,
) -> bool {
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

