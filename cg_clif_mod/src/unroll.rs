//! Loop optimisation on CLIF.
//!
//! Cranelift ships no loop transformations, so a canonical counted loop keeps
//! its full carried dependency chain: `acc = acc*3 + i; i += 1` stays a chain of
//! dependent multiply-adds, one iteration deep. The fix that actually helps is
//! *affine reassociation*: when the loop state is a linear function of itself,
//! its `factor`-fold iterate is also linear and can be evaluated with a handful
//! of multiplies, collapsing `factor` dependent steps into one independent group.
//!
//! This pass recognises a canonical counted loop, derives the affine state
//! update from the body, composes it `factor` times, and rewrites the back edge
//! so one header check may advance the whole state by `factor` iterations at
//! once. A guard keeps the arithmetic exact: the wide step is taken only when
//! every one of those `factor` iterations satisfies the loop test, otherwise the
//! original single step runs. The fallback therefore preserves the tail, and the
//! closed form is exact modulo `2^64` because `iadd`/`imul` wrap.
//!
//! On the scalar benchmark this turns a ~15x LLVM deficit into parity, because
//! LLVM's advantage there is exactly this reassociation (plus unrolling).
//!
//! A body-duplicating unroller is also implemented, but it does not shorten the
//! carried chain and measured slower, so it is off unless `CG_CLIF_UNROLL_CLONE`
//! is set.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::ir::{
    self, condcodes::IntCC, immediates::Imm64, Block, Inst, InstBuilder, Opcode, Value,
};
use rustc_data_structures::fx::{FxHashMap, FxHashSet};

/// Number of loop iterations folded into one guarded step.
pub(crate) const UNROLL_FACTOR: u64 = 128;

/// Upper bound on the induction step we are willing to scale, so that the
/// arithmetic stays well clear of the type's range.
const MAX_DELTA: u64 = 1 << 40;

/// Largest `delta * (factor - 1)` the wide step may use. The guard proves the
/// wide path runs `factor` iterations that all satisfy the loop test by
/// checking `iv + (factor-1)*delta`; that argument only holds while the product
/// itself does not wrap, so it is bounded here and the factor is shrunk to fit.
const MAX_LAST: u64 = 1 << 62;

/// Shrink `factor` until `delta * (factor - 1)` cannot overflow, and report the
/// largest usable factor (>= 2), or `None` when even two iterations overflow.
fn safe_factor(delta: u64, factor: u64) -> Option<u64> {
    let mut factor = factor.min(MAX_LAST / delta.max(1) + 1);
    while factor >= 2 {
        if delta.checked_mul(factor - 1).is_some_and(|last| last <= MAX_LAST) {
            return Some(factor);
        }
        factor -= 1;
    }
    None
}

fn unroll_factor() -> u64 {
    crate::tunables::unroll_factor()
}

struct Plan {
    /// Loop header: the block whose parameters carry the loop state.
    header: Block,
    /// Single-block loop body, branching back to `header`.
    body: Block,
    /// `jump` in the body whose destination is `header`.
    back_edge: Inst,
    /// The induction variable as seen at the top of the header.
    iv: Value,
    /// Loop-invariant bound compared against the induction variable.
    bound: Value,
    /// Comparison direction of the header's continue test.
    cc: IntCC,
    /// Whether the induction variable is the left operand of the compare.
    iv_left: bool,
    /// Constant step applied to the induction variable per iteration.
    delta: u64,
}

/// Collapse blocks that only forward their arguments to another block.
///
/// cg_clif's `if`/`match` lowering leaves a one-instruction latch between the
/// loop body and the header: the body jumps to the latch, the latch passes its
/// parameters straight back to the header. That shape defeats `analyze`, which
/// requires the body itself to carry the back edge, so the loop never unrolls
/// (measured on a min-reduction loop: 2.2x LLVM deficit, unchanged by any
/// unroll factor).
///
/// Threading the jump removes the block: the predecessor's branch is retargeted
/// at the latch's destination with the latch's arguments substituted for its
/// parameters. This is only valid when the latch has a single predecessor, so
/// no other path can observe the removed block, and when every value the latch
/// forwards is available at the predecessor's terminator -- true because a
/// direct successor has nothing defined between them.
fn collapse_forwarders(func: &mut ir::Function) -> bool {
    let mut changed = false;
    while let Some((pred, latch)) = find_forwarder(func) {
        if !thread_forwarder(func, pred, latch) {
            // The candidate was rejected after selection; nothing else can be
            // learned by retrying, and re-finding it would loop forever.
            break;
        }
        if std::env::var_os("CG_CLIF_UNROLL_DEBUG").is_some() {
            eprintln!("[unroll]   collapsed pred={pred:?} latch={latch:?}");
        }
        changed = true;
    }
    changed
}

/// Find a `(predecessor, forwarder)` pair to collapse.
fn find_forwarder(func: &ir::Function) -> Option<(Block, Block)> {
    let dfg = &func.dfg;
    let layout = &func.layout;
    for latch in layout.blocks() {
        // Trivial: nothing but `nop`s and a single trailing `jump`, and nothing
        // that defines a value. cg_clif's `nop` can carry results (it stands in
        // for a MIR statement whose value is discarded), and the jump may then
        // forward one of them; removing the block would dangle that value, which
        // the verifier reports as "defined by block N which is not in the layout".
        let mut jump = None;
        let mut trivial = true;
        for inst in layout.block_insts(latch) {
            if !dfg.inst_results(inst).is_empty() {
                trivial = false;
                break;
            }
            match dfg.insts[inst].opcode() {
                Opcode::Nop => {}
                Opcode::Jump => jump = Some(inst),
                _ => {
                    trivial = false;
                    break;
                }
            }
        }
        if !trivial {
            continue;
        }
        let Some(jump) = jump else { continue };
        let ir::InstructionData::Jump { destination, .. } = dfg.insts[jump] else { continue };
        let target = destination.block(&dfg.value_lists);
        if target == latch || Some(target) == layout.entry_block() {
            continue;
        }
        // Every forwarded argument must survive the latch's removal: block
        // params are substituted with the incoming arguments, so only values
        // *defined by an instruction* in the latch are a hazard, and those are
        // already excluded above by rejecting blocks with results.
        if std::env::var_os("CG_CLIF_UNROLL_DEBUG").is_some() {
            let params: Vec<Value> = dfg.block_params(latch).to_vec();
            let args: Vec<ir::BlockArg> = destination.args(&dfg.value_lists).collect();
            eprintln!("[unroll]     latch {latch:?} params={params:?} fwd={args:?}");
        }

        // Exactly one incoming *edge*, so removing the block cannot strand
        // another edge. Two subtleties:
        //  - Count per edge, not per block: a `brif` whose arms both target the
        //    latch is one predecessor block but two edges.
        //  - Count *every* terminator kind, including `br_table`. Skipping
        //    switch edges here let the pass delete a block that a `match` still
        //    jumped to, which surfaced as "value defined by block N which is not
        //    in the layout" and then as an invalid-value assertion inside the
        //    egraph pass.
        let mut pred = None;
        let mut edges = 0;
        for b in layout.blocks() {
            if b == latch {
                continue;
            }
            for inst in layout.block_insts(b) {
                for call in
                    dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables)
                {
                    if call.block(&dfg.value_lists) == latch {
                        edges += 1;
                        pred = Some(b);
                    }
                }
            }
        }
        if edges != 1 {
            continue;
        }
        let pred = pred.expect("edges == 1");
        // Only thread through a plain jump or conditional branch: rewriting the
        // arguments of a switch arm is not something this pass models.
        let Some(pred_term) = layout.last_inst(pred) else { continue };
        if !matches!(
            dfg.insts[pred_term],
            ir::InstructionData::Jump { .. } | ir::InstructionData::Brif { .. }
        ) {
            continue;
        }
        // Threading into the predecessor would leave a self-loop the unroller
        // rejects anyway; skip it rather than build one.
        if pred == target {
            continue;
        }
        // The latch's parameters must not escape: substituting them with the
        // predecessor's arguments is only sound on this path, so if any other
        // block reads them the block cannot be removed.
        if params_escape(func, latch) {
            continue;
        }
        return Some((pred, latch));
    }
    None
}

/// Whether any block other than `block` reads one of `block`'s parameters.
fn params_escape(func: &ir::Function, block: Block) -> bool {
    let dfg = &func.dfg;
    let params: FxHashSet<Value> = dfg.block_params(block).iter().copied().collect();
    if params.is_empty() {
        return false;
    }
    let reads_param = |v: Value| params.contains(&v) || params.contains(&dfg.resolve_aliases(v));
    for b in func.layout.blocks() {
        if b == block {
            continue;
        }
        for inst in func.layout.block_insts(b) {
            let data = &dfg.insts[inst];
            for call in data.branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                if call.args(&dfg.value_lists).any(|a| match a {
                    ir::BlockArg::Value(v) => reads_param(v),
                    _ => true,
                }) {
                    return true;
                }
            }
            if data.arguments(&dfg.value_lists).iter().any(|v| reads_param(*v)) {
                return true;
            }
        }
    }
    false
}

/// Retarget `pred`'s edge to `latch` at the latch's own destination, replacing
/// the latch's parameters with the arguments `pred` passed them.
fn thread_forwarder(func: &mut ir::Function, pred: Block, latch: Block) -> bool {
    let latch_params: Vec<Value> = func.dfg.block_params(latch).to_vec();
    let latch_term = func.layout.last_inst(latch).expect("forwarder has a jump");
    let (target, latch_args) = match &func.dfg.insts[latch_term] {
        ir::InstructionData::Jump { destination, .. } => (
            destination.block(&func.dfg.value_lists),
            destination.args(&func.dfg.value_lists).collect::<Vec<_>>(),
        ),
        _ => unreachable!("forwarder ends in a jump"),
    };
    let pred_term = func.layout.last_inst(pred).expect("predecessor has a terminator");

    // Arguments the predecessor already passes to the latch, mapped onto the
    // latch's parameters: these are what the latch forwarded onward.
    let incoming: Vec<ir::BlockArg> = match &func.dfg.insts[pred_term] {
        ir::InstructionData::Jump { destination, .. } => {
            destination.args(&func.dfg.value_lists).collect()
        }
        ir::InstructionData::Brif { blocks, .. } => {
            let idx = blocks
                .iter()
                .position(|b| b.block(&func.dfg.value_lists) == latch)
                .expect("pred jumps to latch");
            blocks[idx].args(&func.dfg.value_lists).collect()
        }
        _ => unreachable!("successor edge must be a jump or brif"),
    };
    if incoming.len() != latch_params.len() {
        return false;
    }
    let param_arg: FxHashMap<Value, ir::BlockArg> =
        latch_params.iter().copied().zip(incoming.iter().copied()).collect();

    // Build the replacement arguments *before* touching the pools. Every value
    // must be expressible at `pred`: either a latch parameter (substituted with
    // what `pred` passes in) or a value already defined elsewhere. Anything the
    // latch itself computes would dangle once the block is gone.
    let mut new_args: Vec<ir::BlockArg> = Vec::with_capacity(latch_args.len());
    for a in &latch_args {
        match a {
            ir::BlockArg::Value(v) => {
                let resolved = func.dfg.resolve_aliases(*v);
                if let Some(arg) = param_arg.get(&resolved).or_else(|| param_arg.get(v)) {
                    new_args.push(*arg);
                } else if latch_params.contains(&resolved) || latch_params.contains(v) {
                    return false;
                } else if matches!(func.dfg.value_def(resolved), ir::ValueDef::Result(i, _)
                    if func.layout.inst_block(i) == Some(latch))
                {
                    return false;
                } else {
                    new_args.push(ir::BlockArg::Value(resolved));
                }
            }
            _ => return false,
        }
    }

    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    let new_call = ir::BlockCall::new(target, new_args.iter().copied(), &mut pool);
    match &mut func.dfg.insts[pred_term] {
        ir::InstructionData::Jump { destination, .. } => *destination = new_call,
        ir::InstructionData::Brif { blocks, .. } => {
            let idx = blocks
                .iter()
                .position(|b| b.block(&pool) == latch)
                .expect("pred jumps to latch");
            blocks[idx] = new_call;
        }
        _ => unreachable!("successor edge must be a jump or brif"),
    }
    func.dfg.value_lists = pool;

    // `Layout::remove_block` only *debug*-asserts that the block is empty, so a
    // release build would silently drop the block while its instructions stay in
    // the layout, leaving values pointing at a block that is no longer present.
    for inst in func.layout.block_insts(latch).collect::<Vec<_>>() {
        func.layout.remove_inst(inst);
    }
    func.layout.remove_block(latch);
    true
}

pub(crate) fn run(func: &mut ir::Function) {
    let factor = unroll_factor();
    if factor < 2 {
        return;
    }
    if super::tunables::collapse_forwarders_enabled() {
        collapse_forwarders(func);
    }
    if std::env::var_os("CG_CLIF_UNROLL_DEBUG").is_some() {
        eprintln!("[unroll] func {:?} blocks={}", func.name, func.layout.blocks().count());
        for b in func.layout.blocks() {
            let params: Vec<String> =
                func.dfg.block_params(b).iter().map(|v| format!("{v:?}")).collect();
            eprintln!("[unroll]   {b:?} params=[{}]", params.join(","));
            for i in func.layout.block_insts(b) {
                let extra = match &func.dfg.insts[i] {
                    ir::InstructionData::Jump { destination, .. } => format!(
                        " -> {:?}({:?})",
                        destination.block(&func.dfg.value_lists),
                        destination.args(&func.dfg.value_lists).collect::<Vec<_>>()
                    ),
                    ir::InstructionData::Brif { blocks, .. } => format!(
                        " -> [{:?},{:?}]",
                        blocks[0].block(&func.dfg.value_lists),
                        blocks[1].block(&func.dfg.value_lists)
                    ),
                    _ => String::new(),
                };
                eprintln!("[unroll]   {b:?} {i:?} {}{}", func.dfg.insts[i].opcode(), extra);
            }
        }
    }
    let Some(plan) = analyze(func) else {
        if std::env::var_os("CG_CLIF_UNROLL_DEBUG").is_some() {
            eprintln!("[unroll] -> REJECTED by analyze");
        }
        return;
    };
    // `apply` scales the induction step by `factor`, and the guard it inserts
    // assumes `delta * (factor - 1)` did not wrap. Shrink the factor for large
    // steps so that assumption always holds.
    let Some(factor) = safe_factor(plan.delta, factor) else { return };
    apply(func, &plan, factor);
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

pub(crate) fn const_of(func: &ir::Function, v: Value) -> Option<u64> {
    let v = func.dfg.resolve_aliases(v);
    let ir::ValueDef::Result(inst, 0) = func.dfg.value_def(v) else { return None };
    match func.dfg.insts[inst] {
        ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. } => Some(imm.bits() as u64),
        _ => None,
    }
}

/// Whether `v` is defined inside one of `blocks` (as a parameter or a result).
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

fn analyze(func: &ir::Function) -> Option<Plan> {
    let dfg = &func.dfg;
    let layout = &func.layout;
    let dbg = std::env::var_os("CG_CLIF_UNROLL_DEBUG").is_some();

    for body in layout.blocks() {
        let Some(term) = layout.last_inst(body) else { continue };
        let ir::InstructionData::Jump { destination, opcode: _, .. } = dfg.insts[term] else {
            continue;
        };
        let header = destination.block(&dfg.value_lists);
        if header == body {
            continue;
        }

        // The header must be a two-way branch. Its "then" arm is the body; the
        // other arm leaves the loop.
        let Some(hterm) = layout.last_inst(header) else { continue };
        let ir::InstructionData::Brif { arg: cond, blocks, opcode: _ } = dfg.insts[hterm] else {
            continue;
        };
        if dbg {
            eprintln!(
                "[unroll]   cand body={body:?} header={header:?} brif->[{:?},{:?}]",
                blocks[0].block(&dfg.value_lists),
                blocks[1].block(&dfg.value_lists)
            );
        }
        if blocks[0].block(&dfg.value_lists) != body {
            if dbg { eprintln!("[unroll]     X: then-arm is not body"); }
            continue;
        }

        // The loop must have a single back edge, and it must be the body's.
        let loop_blocks = [header, body];
        let back_edges =
            loop_blocks.iter().filter(|&&b| successors(func, b).contains(&header)).count();
        if back_edges != 1 {
            if dbg { eprintln!("[unroll]     X: back_edges={back_edges}"); }
            continue;
        }

        // The continue test must be `icmp(cc, iv, bound)` with a single header
        // parameter on one side and a loop-invariant value on the other.
        let cond = dfg.resolve_aliases(cond);
        let ir::ValueDef::Result(icmp_inst, 0) = dfg.value_def(cond) else {
            if dbg { eprintln!("[unroll]     X: cond not an inst result"); }
            continue;
        };
        let ir::InstructionData::IntCompare { cond: cc, args, opcode: _ } = dfg.insts[icmp_inst]
        else {
            if dbg { eprintln!("[unroll]     X: cond not icmp: {:?}", dfg.insts[icmp_inst].opcode()); }
            continue;
        };
        // Only increasing loops, so the wide step cannot overshoot the bound.
        let iv_left = match cc {
            IntCC::UnsignedLessThan | IntCC::UnsignedLessThanOrEqual => true,
            IntCC::UnsignedGreaterThan | IntCC::UnsignedGreaterThanOrEqual => false,
            _ => { if dbg { eprintln!("[unroll]     X: cc={cc:?}"); } continue },
        };
        let (lhs, rhs) = (args[0], args[1]);

        let param_of = |v: Value| match dfg.value_def(dfg.resolve_aliases(v)) {
            ir::ValueDef::Param(b, i) if b == header => Some(i),
            _ => None,
        };
        let (iv_idx, iv, bound) = if iv_left {
            let Some(i) = param_of(lhs) else { if dbg { eprintln!("[unroll]     X: lhs not header param"); } continue };
            (i, lhs, rhs)
        } else {
            let Some(i) = param_of(rhs) else { if dbg { eprintln!("[unroll]     X: rhs not header param"); } continue };
            (i, rhs, lhs)
        };
        // The bound must be loop-invariant. A value defined inside the loop is
        // only acceptable when it is a constant, which cg_clif emits inline in
        // the header for a literal trip count.
        let loop_blocks = [header, body];
        if const_of(func, bound).is_none() && defined_in(func, bound, &loop_blocks) {
            if dbg { eprintln!("[unroll]     X: bound not invariant"); }
            continue;
        }

        // The induction step is the back edge's argument for the induction
        // parameter: it must be `iv + <small positive constant>`.
        let ir::InstructionData::Jump { destination: back_call, opcode: _, .. } = dfg.insts[term]
        else {
            unreachable!()
        };
        let args: Vec<ir::BlockArg> = back_call.args(&dfg.value_lists).collect();
        if args.len() != dfg.block_params(header).len() {
            if dbg { eprintln!("[unroll]     X: arity mismatch"); }
            continue;
        }
        let ir::BlockArg::Value(step_val) = args[iv_idx] else { if dbg { eprintln!("[unroll]     X: step not value"); } continue };
        let step_val = dfg.resolve_aliases(step_val);
        let ir::ValueDef::Result(step_inst, 0) = dfg.value_def(step_val) else { if dbg { eprintln!("[unroll]     X: step not inst result"); } continue };
        let ir::InstructionData::Binary { opcode: Opcode::Iadd, args: sargs, .. } =
            dfg.insts[step_inst]
        else {
            if dbg { eprintln!("[unroll]     X: step not iadd"); }
            continue;
        };
        let iv = dfg.resolve_aliases(iv);
        let delta = match (const_of(func, sargs[0]), const_of(func, sargs[1])) {
            (None, Some(d)) if dfg.resolve_aliases(sargs[0]) == iv => d,
            (Some(d), None) if dfg.resolve_aliases(sargs[1]) == iv => d,
            _ => { if dbg { eprintln!("[unroll]     X: step not iv+const: sargs={sargs:?} iv={iv:?}"); } continue },
        };
        if delta == 0 || delta > MAX_DELTA {
            if dbg { eprintln!("[unroll]     X: delta={delta}"); }
            continue;
        }

        return Some(Plan { header, body, back_edge: term, iv, bound, cc, iv_left, delta });
    }
    None
}

/// Copy the body's instructions into `dst`, remapping each header parameter to
/// the corresponding `dst` parameter. Instructions that use header *parameters*
/// (rather than header-local temporaries) therefore read the clone's state.
fn clone_body(func: &mut ir::Function, plan: &Plan, dst: Block) {
    let header_params: Vec<Value> = func.dfg.block_params(plan.header).to_vec();
    let dst_params: Vec<Value> = func.dfg.block_params(dst).to_vec();

    // Map header parameters to the clone's parameters, and every alias of a
    // header parameter to the same clone parameter. cg_clif routinely emits
    // `v6 -> v2` aliases, and the body reads the loop state through those, so
    // resolving them here is what makes the clone read its own state.
    let param_map: FxHashMap<Value, Value> =
        header_params.iter().copied().zip(dst_params.iter().copied()).collect();
    let mut vmap: FxHashMap<Value, Value> = FxHashMap::default();
    for v in func.dfg.values() {
        let resolved = func.dfg.resolve_aliases(v);
        if let Some(&d) = param_map.get(&resolved) {
            vmap.insert(v, d);
        }
    }

    let insts: Vec<Inst> = func.layout.block_insts(plan.body).collect();
    for &old_inst in &insts {
        let mut data = func.dfg.insts[old_inst];
        {
            let mut value_lists = std::mem::take(&mut func.dfg.value_lists);
            let mut jump_tables = std::mem::take(&mut func.dfg.jump_tables);
            let mut exception_tables = std::mem::take(&mut func.dfg.exception_tables);
            // `InstructionData` is `Copy` and its `BlockCall`s share the value
            // pool with the original instruction. Deep-clone them first, or
            // remapping the clone's branch arguments would corrupt the original.
            for call in data.branch_destination_mut(&mut jump_tables, &mut exception_tables) {
                *call = call.deep_clone(&mut value_lists);
            }
            data.map_values(&mut value_lists, &mut jump_tables, &mut exception_tables, |v| {
                *vmap.get(&v).unwrap_or(&v)
            });
            func.dfg.value_lists = value_lists;
            func.dfg.jump_tables = jump_tables;
            func.dfg.exception_tables = exception_tables;
        }
        let ctrl_typevar = func.dfg.ctrl_typevar(old_inst);
        let new_inst = func.dfg.make_inst(data);
        func.dfg.make_inst_results(new_inst, ctrl_typevar);
        let old_results = func.dfg.inst_results(old_inst).to_vec();
        let new_results = func.dfg.inst_results(new_inst).to_vec();
        for (o, n) in old_results.iter().zip(&new_results) {
            vmap.insert(*o, *n);
        }
        func.layout.append_inst(new_inst, dst);
    }
}

/// Rewrite the branch destination of a `jump`, keeping its arguments.
fn retarget(func: &mut ir::Function, inst: Inst, target: Block) {
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    if let ir::InstructionData::Jump { destination, .. } = &mut func.dfg.insts[inst] {
        let args: Vec<ir::BlockArg> = destination.args(&pool).collect();
        *destination = ir::BlockCall::new(target, args, &mut pool);
    }
    func.dfg.value_lists = pool;
}

/// A value expressed as `sum(coeffs[i] * header_param[i]) + c` over the
/// integers modulo `2^64`.
#[derive(Clone)]
struct Affine {
    coeffs: Vec<u64>,
    c: u64,
}

/// Compute the affine form of every instruction result in `plan.body`, starting
/// from the header parameters as the basis. Returns `None` if the body is not a
/// pure affine update of the loop state (it reads or writes memory, calls out, or
/// uses an operation we cannot model), which keeps the reassociation sound.
fn analyze_affine(func: &ir::Function, plan: &Plan) -> Option<Vec<Option<Affine>>> {
    let dfg = &func.dfg;
    let n = dfg.block_params(plan.header).len();

    // The closed form uses 64-bit arithmetic; a header carrying anything else
    // (a pointer, a float) is left alone.
    if !dfg.block_params(plan.header).iter().all(|&p| dfg.value_type(p) == ir::types::I64) {
        return None;
    }

    let zero = Affine { coeffs: vec![0; n], c: 0 };
    let basis = |i: usize| {
        let mut a = zero.clone();
        a.coeffs[i] = 1;
        a
    };
    // Only 64-bit induction/accumulator arithmetic is modelled, to keep the
    // wrapping semantics of the closed form exactly right.
    let is_i64 = |v: Value| dfg.value_type(dfg.resolve_aliases(v)) == ir::types::I64;

    let mut forms: Vec<Option<Affine>> = vec![None; dfg.num_values()];
    for (i, &p) in dfg.block_params(plan.header).iter().enumerate() {
        forms[p.as_u32() as usize] = Some(basis(i));
    }

    let add = |a: &Affine, b: &Affine| {
        let mut r = a.clone();
        for (x, y) in r.coeffs.iter_mut().zip(&b.coeffs) {
            *x = x.wrapping_add(*y);
        }
        r.c = r.c.wrapping_add(b.c);
        r
    };
    let scale = |a: &Affine, k: u64| {
        let mut r = a.clone();
        for x in r.coeffs.iter_mut() {
            *x = x.wrapping_mul(k);
        }
        r.c = r.c.wrapping_mul(k);
        r
    };
    let constant = |c: u64| Affine { coeffs: vec![0; n], c };
    let as_const = |a: &Option<Affine>| -> Option<u64> {
        let a = a.as_ref()?;
        if a.coeffs.iter().all(|&x| x == 0) {
            Some(a.c)
        } else {
            None
        }
    };

    for inst in func.layout.block_insts(plan.body) {
        let results = dfg.inst_results(inst).to_vec();
        let value = match dfg.insts[inst] {
            ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. }
                if is_i64(results[0]) =>
            {
                Some(constant(imm.bits() as u64))
            }
            ir::InstructionData::Binary { opcode: Opcode::Iadd, args, .. } if is_i64(args[0]) => {
                let (a, b) = (
                    forms[dfg.resolve_aliases(args[0]).as_u32() as usize].clone(),
                    forms[dfg.resolve_aliases(args[1]).as_u32() as usize].clone(),
                );
                match (a, b) {
                    (Some(a), Some(b)) => Some(add(&a, &b)),
                    _ => None,
                }
            }
            ir::InstructionData::Binary { opcode: Opcode::Imul, args, .. } if is_i64(args[0]) => {
                let (a, b) = (
                    forms[dfg.resolve_aliases(args[0]).as_u32() as usize].clone(),
                    forms[dfg.resolve_aliases(args[1]).as_u32() as usize].clone(),
                );
                match (as_const(&a), as_const(&b)) {
                    (Some(k), _) => b.map(|b| scale(&b, k)),
                    (_, Some(k)) => a.map(|a| scale(&a, k)),
                    _ => None,
                }
            }
            // Any memory access, call or unmodelled op aborts the rewrite.
            _ => None,
        };
        for r in results {
            forms[r.as_u32() as usize] = value.clone();
        }
    }
    Some(forms)
}

/// Compose the affine state update `factor` times and return the matrix mapping
/// the header parameters to their values after `factor` iterations.
fn power(func: &ir::Function, plan: &Plan, forms: &[Option<Affine>], factor: u64) -> Option<Vec<Affine>> {
    let dfg = &func.dfg;
    let n = dfg.block_params(plan.header).len();
    let back_args: Vec<Value> = match dfg.insts[plan.back_edge] {
        ir::InstructionData::Jump { destination, .. } => destination
            .args(&dfg.value_lists)
            .filter_map(|a| a.as_value())
            .collect(),
        _ => return None,
    };
    if back_args.len() != n {
        return None;
    }
    let step: Vec<Affine> = back_args
        .iter()
        .map(|&v| forms[dfg.resolve_aliases(v).as_u32() as usize].clone())
        .collect::<Option<Vec<_>>>()?;

    // Row `j` maps state -> state_j after one iteration. Iterate `factor` times.
    let mut rows = step.clone();
    for _ in 1..factor {
        let cur = rows.clone();
        rows = cur
            .iter()
            .map(|row| {
                let mut acc = Affine { coeffs: vec![0; n], c: row.c };
                for (i, &k) in row.coeffs.iter().enumerate() {
                    if k != 0 {
                        let mut scaled = step[i].clone();
                        for x in scaled.coeffs.iter_mut() {
                            *x = x.wrapping_mul(k);
                        }
                        scaled.c = scaled.c.wrapping_mul(k);
                        for (a, b) in acc.coeffs.iter_mut().zip(&scaled.coeffs) {
                            *a = a.wrapping_add(*b);
                        }
                        acc.c = acc.c.wrapping_add(scaled.c);
                    }
                }
                acc
            })
            .collect();
    }
    Some(rows)
}

fn apply(func: &mut ir::Function, plan: &Plan, factor: u64) {
    // The profitable transformation is affine reassociation: if the loop state
    // is a linear function of itself, its `factor`-fold iterate is also linear
    // and can be evaluated in a handful of multiplies, collapsing the carried
    // dependency chain. Duplicating the body cannot achieve that.
    if let Some(forms) = analyze_affine(func, plan) {
        if let Some(rows) = power(func, plan, &forms, factor) {
            apply_reassoc(func, plan, factor, &rows);
            return;
        }
    }
    if crate::tunables::unroll_clone() {
        apply_clone(func, plan, factor);
    }
}

/// Rewrite the back edge so one header check may advance the loop state by
/// `factor` iterations at once, using the closed-form iterate `rows`, guarded so
/// a short tail falls back to the original single step.
fn apply_reassoc(func: &mut ir::Function, plan: &Plan, factor: u64, rows: &[Affine]) {
    let delta = plan.delta;
    let last = delta.checked_mul(factor - 1).expect("safe_factor bounds the step");
    let back_edge = plan.back_edge;

    // Arguments of the original back edge: already the single-step values.
    let single_args: Vec<ir::BlockArg> = match func.dfg.insts[back_edge] {
        ir::InstructionData::Jump { destination, .. } => {
            destination.args(&func.dfg.value_lists).collect()
        }
        _ => unreachable!(),
    };

    let header_params: Vec<Value> = func.dfg.block_params(plan.header).to_vec();
    let header = plan.header;

    // Build the guard and the composed state in front of the back edge.
    let (guard, wide_args) = {
        let mut pos = FuncCursor::new(func);
        pos.goto_inst(back_edge);

        // Wide path only when all `factor` iterations satisfy the loop test:
        // `iv + (factor-1)*delta` must still pass, and must not have wrapped.
        let iv = plan.iv;
        let iv_last = pos.ins().iadd_imm_s(iv, Imm64::new(last as i64));
        let no_wrap = pos.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, iv_last, iv);
        let last_ok = if plan.iv_left {
            pos.ins().icmp(plan.cc, iv_last, plan.bound)
        } else {
            pos.ins().icmp(plan.cc, plan.bound, iv_last)
        };
        let guard = pos.ins().band(no_wrap, last_ok);

        let mut wide_args = Vec::with_capacity(rows.len());
        for row in rows {
            let mut acc: Option<Value> = None;
            for (i, &k) in row.coeffs.iter().enumerate() {
                if k == 0 {
                    continue;
                }
                let term = if k == 1 {
                    header_params[i]
                } else {
                    let c = pos.ins().iconst(ir::types::I64, Imm64::new(k as i64));
                    pos.ins().imul(header_params[i], c)
                };
                acc = Some(match acc {
                    Some(a) => pos.ins().iadd(a, term),
                    None => term,
                });
            }
            let c = if row.c != 0 {
                Some(pos.ins().iconst(ir::types::I64, Imm64::new(row.c as i64)))
            } else {
                None
            };
            let value = match (acc, c) {
                (Some(a), Some(c)) => pos.ins().iadd(a, c),
                (Some(a), None) => a,
                (None, Some(c)) => c,
                (None, None) => pos.ins().iconst(ir::types::I64, Imm64::new(0)),
            };
            wide_args.push(ir::BlockArg::Value(value));
        }
        (guard, wide_args)
    };

    // Replace the back edge with a guard branch on the composed state.
    func.layout.remove_inst(back_edge);
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    let data = ir::InstructionData::Brif {
        opcode: Opcode::Brif,
        arg: guard,
        blocks: [
            ir::BlockCall::new(header, wide_args, &mut pool),
            ir::BlockCall::new(header, single_args, &mut pool),
        ],
    };
    let inst = func.dfg.make_inst(data);
    func.layout.append_inst(inst, plan.body);
    func.dfg.value_lists = pool;
}

/// Body-duplicating unroll: correct, but it does not shorten the carried
/// dependency chain, so it is only used as a fallback when explicitly enabled.
fn apply_clone(func: &mut ir::Function, plan: &Plan, factor: u64) {
    let delta = plan.delta;
    let last = delta.checked_mul(factor - 1).expect("safe_factor bounds the step");

    // Create `factor - 1` clone blocks with the header's parameter types.
    let header_tys: Vec<_> =
        func.dfg.block_params(plan.header).iter().map(|&p| func.dfg.value_type(p)).collect();
    let mut clones = Vec::new();
    for _ in 0..factor - 1 {
        let nb = func.dfg.make_block();
        func.layout.append_block(nb);
        for ty in &header_tys {
            func.dfg.append_block_param(nb, *ty);
        }
        clones.push(nb);
    }

    for &nb in &clones {
        clone_body(func, plan, nb);
    }

    // Guard at the top of the body: the wide path is taken only when all `factor`
    // iterations satisfy the loop test. `iv + last` must also not wrap, which
    // would otherwise make the test see a small value.
    let guard = {
        let iv = plan.iv;
        let bound = plan.bound;
        let mut pos = FuncCursor::new(func);
        pos.goto_first_insertion_point(plan.body);
        let iv_last = pos.ins().iadd_imm_s(iv, Imm64::new(last as i64));
        let no_wrap = pos.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, iv_last, iv);
        let last_ok = if plan.iv_left {
            pos.ins().icmp(plan.cc, iv_last, bound)
        } else {
            pos.ins().icmp(plan.cc, bound, iv_last)
        };
        pos.ins().band(no_wrap, last_ok)
    };

    // Chain the clones between the body and the header. Every terminator keeps
    // its arguments, which the body's own induction arithmetic has already
    // advanced, so the clones walk `iv`, `iv + delta`, ... automatically.
    for k in 0..clones.len() {
        let term = func.layout.last_inst(clones[k]).unwrap();
        if k + 1 < clones.len() {
            retarget(func, term, clones[k + 1]);
        }
    }
    let first = clones[0];
    let body_term = plan.back_edge;
    let body_args: Vec<ir::BlockArg> = match func.dfg.insts[body_term] {
        ir::InstructionData::Jump { destination, .. } => {
            destination.args(&func.dfg.value_lists).collect()
        }
        _ => unreachable!(),
    };

    // Replace the body's back edge with a guard branch: the wide path runs the
    // clones (covering `factor` iterations), the fallback takes a single step.
    func.layout.remove_inst(body_term);
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    let data = ir::InstructionData::Brif {
        opcode: Opcode::Brif,
        arg: guard,
        blocks: [
            ir::BlockCall::new(first, body_args.clone(), &mut pool),
            ir::BlockCall::new(plan.header, body_args, &mut pool),
        ],
    };
    let inst = func.dfg.make_inst(data);
    func.layout.append_inst(inst, plan.body);
    func.dfg.value_lists = pool;
}
