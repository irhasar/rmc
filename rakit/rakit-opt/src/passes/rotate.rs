//! Loop rotation (bottom-tested loops).
//!
//! Rakit lowers a counted or guarded loop to a header that tests the condition
//! at the top and a latch that jumps back to it:
//!
//! ```text
//! header(p):  cond = icmp(cc, ...);  brif cond, body(p), exit(p)
//! body(q):    ...                     jump header(next)
//! ```
//!
//! Every iteration therefore executes the test *and* an unconditional jump back
//! to the header. LLVM's loop-rotate instead makes the body its own back edge,
//! testing at the bottom:
//!
//! ```text
//! header(p):  cond = icmp(cc, ...);           brif cond, body(p), exit(p)   ; once
//! body(q):    ...  cond2 = icmp(cc, ...next); brif cond2, body(next), exit(next)
//! ```
//!
//! That removes the header re-entry and the unconditional back-jump from the hot
//! path. It matters most on call-bound loops, where branch throughput competes
//! with call overhead (the recursive-to-loop `fib` kernel).
//!
//! The transform is local and conservative. It only fires when the header is a
//! two-way `brif` on a single `icmp`, exactly one latch (inside the loop) jumps
//! back to the header, the body arm passes the header parameters through
//! unchanged (so the latch can re-enter the body with its own next-state
//! arguments), and every operand the recomputed test and exit edge need is
//! either a header parameter (replaced by the latch's argument), a constant, or
//! a value defined outside the loop.
//!
//! By default only call-bound loops are rotated: that is where the saved branch
//! competes with call overhead, and it leaves pure arithmetic loops to the
//! unroller. `RAKIT_ROTATE_ALL=1` widens it to every matching loop.

use std::collections::HashSet as FxHashSet;

use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::condcodes::IntCC;
use rakit_codegen::ir::{self, Block, BlockArg, Inst, InstBuilder, Opcode, Value};

/// Everything the rewrite needs, gathered while `func` is only borrowed shared.
struct Prepared {
    latch_blk: Block,
    latch_term: Inst,
    body_target: Block,
    exit_target: Block,
    body_idx: usize,
    cc: IntCC,
    new_l: Value,
    new_r: Value,
    back_vals: Vec<Value>,
    new_exit_args: Vec<BlockArg>,
}

pub fn run(func: &mut ir::Function) {
    if !crate::tunables::rotate_enabled() {
        return;
    }
    // Only a block that some `jump` targets can be a loop header, so collect
    // those once and skip the reachability walk for every other branch.
    let mut jump_targets: FxHashSet<Block> = FxHashSet::default();
    for b in func.layout.blocks() {
        if let Some(t) = func.layout.last_inst(b) {
            if let ir::InstructionData::Jump { destination, .. } = func.dfg.insts[t] {
                jump_targets.insert(destination.block(&func.dfg.value_lists));
            }
        }
    }
    if jump_targets.is_empty() {
        return;
    }
    let blocks: Vec<Block> = func.layout.blocks().collect();
    for h in blocks {
        if jump_targets.contains(&h) {
            let _ = try_rotate(func, h);
        }
    }
}

fn successors(func: &ir::Function, block: Block) -> Vec<Block> {
    let dfg = &func.dfg;
    let mut out = Vec::new();
    for inst in func.layout.block_insts(block) {
        for call in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
            out.push(call.block(&dfg.value_lists));
        }
    }
    out
}

/// Whether `goal` is reachable from `start` without expanding `avoid`.
fn reachable_avoid(func: &ir::Function, start: Block, goal: Block, avoid: Block) -> bool {
    if start == goal {
        return true;
    }
    if start == avoid {
        return false;
    }
    let mut seen: FxHashSet<Block> = FxHashSet::default();
    let mut stack = vec![start];
    seen.insert(start);
    while let Some(b) = stack.pop() {
        for s in successors(func, b) {
            if s == goal {
                return true;
            }
            if s == avoid || !seen.insert(s) {
                continue;
            }
            stack.push(s);
        }
    }
    false
}

/// Blocks reachable from `start` without expanding `avoid`, plus `avoid` itself.
fn loop_region(func: &ir::Function, start: Block, avoid: Block) -> Vec<Block> {
    let mut seen: FxHashSet<Block> = FxHashSet::default();
    let mut stack = vec![start];
    seen.insert(start);
    while let Some(b) = stack.pop() {
        for s in successors(func, b) {
            if s == avoid || !seen.insert(s) {
                continue;
            }
            stack.push(s);
        }
    }
    let mut out: Vec<Block> = seen.into_iter().collect();
    out.push(avoid);
    out
}

fn defined_in(func: &ir::Function, v: Value, blocks: &[Block]) -> bool {
    let dfg = &func.dfg;
    match dfg.value_def(dfg.resolve_aliases(v)) {
        ir::ValueDef::Param(b, _) => blocks.contains(&b),
        ir::ValueDef::Result(inst, _) => {
            func.layout.inst_block(inst).is_some_and(|b| blocks.contains(&b))
        }
        ir::ValueDef::Union(..) => true,
    }
}

fn const_of(func: &ir::Function, v: Value) -> Option<u64> {
    let v = func.dfg.resolve_aliases(v);
    let ir::ValueDef::Result(inst, 0) = func.dfg.value_def(v) else { return None };
    match func.dfg.insts[inst] {
        ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. } => Some(imm.bits() as u64),
        _ => None,
    }
}

/// Whether the loop region reads a value the header defines non-constantly.
///
/// Rakit's frontend lets a block read a dominating block's values, so the loop
/// body often uses the header's block parameters directly instead of the body
/// block's own parameters. After rotation the header runs only once, so any
/// such use would keep the first iteration's value. Reject the loop when either
/// arm's subgraph reads a header parameter or a non-constant header result.
fn header_values_escape(
    func: &ir::Function,
    h: Block,
    reach: &FxHashSet<Block>,
    params: &[Value],
) -> bool {
    let dfg = &func.dfg;
    let pool = &dfg.value_lists;
    let mut hdr: FxHashSet<Value> = FxHashSet::default();
    for &p in params {
        hdr.insert(dfg.resolve_aliases(p));
    }
    for inst in func.layout.block_insts(h) {
        for &r in dfg.inst_results(inst) {
            let rr = dfg.resolve_aliases(r);
            if const_of(func, rr).is_none() {
                hdr.insert(rr);
            }
        }
    }
    if hdr.is_empty() {
        return false;
    }
    for &b in reach {
        for inst in func.layout.block_insts(b) {
            for &v in dfg.inst_args(inst) {
                if hdr.contains(&dfg.resolve_aliases(v)) {
                    return true;
                }
            }
            for call in
                dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables)
            {
                for arg in call.args(pool) {
                    if let BlockArg::Value(v) = arg {
                        if hdr.contains(&dfg.resolve_aliases(v)) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Whether the header does anything besides its terminating `brif`.
///
/// Rotation expands the header's `brif` at the latch, so any other effect in
/// the header (a call, a store, a trapping arithmetic op, a volatile load) would
/// be skipped on every iteration after the first. Only pure computation is
/// allowed; its results are still validated by `header_values_escape`.
fn header_has_effects(func: &ir::Function, h: Block) -> bool {
    let last = func.layout.last_inst(h);
    for inst in func.layout.block_insts(h) {
        if Some(inst) == last {
            continue;
        }
        let data = &func.dfg.insts[inst];
        let op = data.opcode();
        let pure_load = matches!(
            data,
            ir::InstructionData::Load { opcode: Opcode::Load, flags, .. } if {
                let f = &func.dfg.mem_flags[*flags];
                f.readonly() && f.notrap() && f.can_move()
            }
        );
        let effects = op.is_call()
            || op.is_branch()
            || op.is_terminator()
            || op.is_return()
            || op.can_trap()
            || op.other_side_effects()
            || op.can_store()
            || (op.can_load() && !pure_load);
        if effects {
            return true;
        }
    }
    false
}

fn body_has_call(func: &ir::Function, region: &[Block]) -> bool {
    region.iter().any(|&b| {
        func.layout
            .block_insts(b)
            .any(|i| matches!(func.dfg.insts[i].opcode(), Opcode::Call | Opcode::CallIndirect))
    })
}

fn prepare(func: &ir::Function, h: Block) -> Option<Prepared> {
    let dfg = &func.dfg;
    let pool = &dfg.value_lists;
    let term = func.layout.last_inst(h)?;
    let ir::InstructionData::Brif { arg: cond, blocks, opcode: _ } = dfg.insts[term] else {
        return None;
    };
    let targets = [blocks[0].block(pool), blocks[1].block(pool)];
    let arms = [
        blocks[0].args(pool).collect::<Vec<BlockArg>>(),
        blocks[1].args(pool).collect::<Vec<BlockArg>>(),
    ];
    if targets[0] == h || targets[1] == h || targets[0] == targets[1] {
        return None;
    }

    // The loop body is everything reachable from the two arms without
    // re-entering the header. The preheader (which jumps *into* the header from
    // outside) is not in that set, so it cannot be mistaken for a latch.
    let mut reach: FxHashSet<Block> = FxHashSet::default();
    let mut stack = vec![targets[0], targets[1]];
    reach.extend(stack.iter().copied());
    while let Some(b) = stack.pop() {
        for s in successors(func, b) {
            if s != h && reach.insert(s) {
                stack.push(s);
            }
        }
    }

    // Exactly one latch inside the loop jumping back to the header.
    let mut latch: Option<(Block, Inst)> = None;
    for &b in &reach {
        let Some(t) = func.layout.last_inst(b) else { continue };
        if let ir::InstructionData::Jump { destination, .. } = dfg.insts[t] {
            if destination.block(pool) == h {
                if latch.is_some() {
                    return None;
                }
                latch = Some((b, t));
            }
        }
    }
    let (latch_blk, latch_term) = latch?;

    // The body arm is the one that reaches the latch without passing the header.
    let body_idx = match (
        reachable_avoid(func, targets[0], latch_blk, h),
        reachable_avoid(func, targets[1], latch_blk, h),
    ) {
        (true, false) => 0,
        (false, true) => 1,
        _ => return None,
    };
    let body_target = targets[body_idx];
    let exit_target = targets[1 - body_idx];

    let params: Vec<Value> = dfg.block_params(h).to_vec();
    if params.is_empty() || arms[body_idx].len() != params.len() {
        return None;
    }
    if dfg.block_params(body_target).len() != params.len() {
        return None;
    }
    // The body arm must pass the header parameters through unchanged, and the
    // body's own parameters must mirror them, so the latch's arguments can be
    // fed straight back into the body.
    for (i, arg) in arms[body_idx].iter().enumerate() {
        let BlockArg::Value(v) = *arg else { return None };
        if dfg.resolve_aliases(v) != dfg.resolve_aliases(params[i]) {
            return None;
        }
        if dfg.value_type(params[i]) != dfg.value_type(dfg.block_params(body_target)[i]) {
            return None;
        }
    }

    let ir::InstructionData::Jump { destination: back, .. } = dfg.insts[latch_term] else {
        return None;
    };
    let back_args: Vec<BlockArg> = back.args(pool).collect();
    if back_args.len() != params.len() {
        return None;
    }
    let mut back_vals = Vec::with_capacity(back_args.len());
    for arg in &back_args {
        let BlockArg::Value(v) = *arg else { return None };
        back_vals.push(v);
    }

    if header_has_effects(func, h) || header_values_escape(func, h, &reach, &params) {
        return None;
    }
    let region = loop_region(func, body_target, h);
    if !crate::tunables::rotate_all() && !body_has_call(func, &region) {
        return None;
    }

    // The header condition must be a single integer comparison.
    let ir::ValueDef::Result(cond_inst, 0) = dfg.value_def(dfg.resolve_aliases(cond)) else {
        return None;
    };
    let ir::InstructionData::IntCompare { cond: cc, args: cargs, opcode: _ } = dfg.insts[cond_inst]
    else {
        return None;
    };

    let subst = |v: Value| -> Option<Value> {
        let rv = dfg.resolve_aliases(v);
        if let ir::ValueDef::Param(b, i) = dfg.value_def(rv) {
            if b == h {
                return Some(back_vals[i]);
            }
        }
        // A constant is identical on every iteration, so the copy defined in the
        // header dominates the latch and can be reused. A non-constant header
        // instruction would hold the first iteration's value, so it is refused.
        if const_of(func, rv).is_some() || !defined_in(func, rv, &region) {
            Some(rv)
        } else {
            None
        }
    };
    let new_l = subst(cargs[0])?;
    let new_r = subst(cargs[1])?;

    let mut new_exit_args = Vec::with_capacity(arms[1 - body_idx].len());
    for arg in &arms[1 - body_idx] {
        let BlockArg::Value(v) = *arg else { return None };
        new_exit_args.push(BlockArg::Value(subst(v)?));
    }

    Some(Prepared {
        latch_blk,
        latch_term,
        body_target,
        exit_target,
        body_idx,
        cc,
        new_l,
        new_r,
        back_vals,
        new_exit_args,
    })
}

fn try_rotate(func: &mut ir::Function, h: Block) -> bool {
    let Some(p) = prepare(func, h) else { return false };

    let cond2 = {
        let mut pos = FuncCursor::new(func).at_inst(p.latch_term);
        pos.ins().icmp(p.cc, p.new_l, p.new_r)
    };

    func.layout.remove_inst(p.latch_term);
    let mut vpool = std::mem::take(&mut func.dfg.value_lists);
    let body_call = ir::BlockCall::new(
        p.body_target,
        p.back_vals.iter().map(|v| BlockArg::Value(*v)),
        &mut vpool,
    );
    let exit_call =
        ir::BlockCall::new(p.exit_target, p.new_exit_args.iter().copied(), &mut vpool);
    let arms = if p.body_idx == 0 {
        [body_call, exit_call]
    } else {
        [exit_call, body_call]
    };
    let data = ir::InstructionData::Brif { opcode: Opcode::Brif, arg: cond2, blocks: arms };
    let inst = func.dfg.make_inst(data);
    func.layout.append_inst(inst, p.latch_blk);
    func.dfg.value_lists = vpool;
    true
}
