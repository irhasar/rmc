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

/// Widest step the partial-reassociation path will clone. Each iteration costs a
/// body copy, and the win saturates once the multiply chain is broken, so a small
/// factor is enough.
pub(crate) const PARTIAL_FACTOR: u64 = 6;

/// Partial factor for a loop whose forcing term is a recomputable atom (`i | 1`).
/// Rebuilding each atom per step costs O(factor^2) multiplies, so a large factor
/// is counter-productive; 2 keeps every multiply shallow and independent.
pub(crate) const ATOM_FACTOR: u64 = 3;

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
fn collapse_forwarders(func: &mut ir::Function) {
    let mut ctx = Forwarders::new(func);
    while let Some((pred, latch, target)) = ctx.find(func) {
        if !thread_forwarder(func, pred, latch) {
            // The candidate was rejected after selection; nothing else can be
            // learned by retrying, and re-finding it would loop forever.
            break;
        }
        if std::env::var_os("CG_CLIF_UNROLL_DEBUG").is_some() {
            eprintln!("[unroll]   collapsed pred={pred:?} latch={latch:?}");
        }
        ctx.removed(pred, latch, target);
    }
}

/// Facts about a function's blocks that the forwarder search needs, gathered
/// once instead of once per candidate.
///
/// The search used to rebuild these on every successful collapse, which is
/// quadratic in the block count: a single large codegen unit spent ~19 ms of
/// compiler time rediscovering the same edge counts 1400 times.
struct Forwarders {
    /// Incoming edges to a block from *other* blocks.
    edges: FxHashMap<Block, usize>,
    /// The source of the single incoming edge, when there is exactly one.
    pred_of: FxHashMap<Block, Block>,
    /// How many distinct blocks read each value.
    read_blocks: FxHashMap<Value, usize>,
    /// Blocks whose branch passes a non-`Value` block argument.
    has_non_value_arg: FxHashSet<Block>,
    /// Blocks that are a candidate forwarder, in layout order.
    candidates: Vec<Block>,
}

impl Forwarders {
    fn new(func: &ir::Function) -> Self {
        let dfg = &func.dfg;
        let layout = &func.layout;
        let mut edges: FxHashMap<Block, usize> = FxHashMap::default();
        let mut pred_of: FxHashMap<Block, Block> = FxHashMap::default();
        let mut read_blocks: FxHashMap<Value, usize> = FxHashMap::default();
        let mut has_non_value_arg: FxHashSet<Block> = FxHashSet::default();
        let mut candidates = Vec::new();
        for b in layout.blocks() {
            let mut read_here: FxHashSet<Value> = FxHashSet::default();
            let mut trivial = true;
            let mut jump = None;
            for inst in layout.block_insts(b) {
                if !dfg.inst_results(inst).is_empty() {
                    trivial = false;
                }
                match dfg.insts[inst].opcode() {
                    Opcode::Nop => {}
                    Opcode::Jump => jump = Some(inst),
                    _ => trivial = false,
                }
                let data = &dfg.insts[inst];
                for call in data.branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                    let target = call.block(&dfg.value_lists);
                    if target != b {
                        *edges.entry(target).or_insert(0) += 1;
                        pred_of.insert(target, b);
                    }
                    for a in call.args(&dfg.value_lists) {
                        match a {
                            ir::BlockArg::Value(v) => {
                                read_here.insert(dfg.resolve_aliases(v));
                            }
                            _ => {
                                has_non_value_arg.insert(target);
                            }
                        }
                    }
                }
                for &v in data.arguments(&dfg.value_lists) {
                    read_here.insert(dfg.resolve_aliases(v));
                }
            }
            for v in read_here {
                *read_blocks.entry(v).or_insert(0) += 1;
            }
            if trivial && jump.is_some() {
                candidates.push(b);
            }
        }
        Forwarders { edges, pred_of, read_blocks, has_non_value_arg, candidates }
    }

    /// Drop `latch`, which `thread_forwarder` has just retargeted away.
    ///
    /// `latch` leaves the layout, so its own edge to `target` disappears, and
    /// the predecessor's edge to `latch` now points at `target` instead. The
    /// edge count of `target` is therefore unchanged; only its recorded source
    /// moves. Getting this wrong makes the next `find` see a phantom second
    /// predecessor and stop collapsing early.
    fn removed(&mut self, pred: Block, latch: Block, target: Block) {
        self.candidates.retain(|&b| b != latch);
        self.edges.remove(&latch);
        self.pred_of.remove(&latch);
        self.pred_of.insert(target, pred);
    }

    /// Find a `(predecessor, forwarder, target)` triple to collapse.
    fn find(&self, func: &ir::Function) -> Option<(Block, Block, Block)> {
        let dfg = &func.dfg;
        let layout = &func.layout;
        for &latch in &self.candidates {
            // Trivial: nothing but `nop`s and a single trailing `jump`, and
            // nothing that defines a value. cg_clif's `nop` can carry results (it
            // stands in for a MIR statement whose value is discarded), and the
            // jump may then forward one of them; removing the block would dangle
            // that value, which the verifier reports as "defined by block N which
            // is not in the layout". `Forwarders::new` already filtered on that.
            if !layout.is_block_inserted(latch) {
                continue;
            }
            let Some(jump) = layout.last_inst(latch) else { continue };
            let ir::InstructionData::Jump { destination, .. } = dfg.insts[jump] else { continue };
            let target = destination.block(&dfg.value_lists);
            if target == latch || Some(target) == layout.entry_block() {
                continue;
            }
            // Exactly one incoming *edge*, so removing the block cannot strand
            // another edge. `Forwarders::new` counts per edge, not per block, and
            // includes `br_table` arms.
            if self.edges.get(&latch).copied().unwrap_or(0) != 1 {
                continue;
            }
            let pred = self.pred_of[&latch];
            // Only thread through a plain jump or conditional branch: rewriting
            // the arguments of a switch arm is not something this pass models.
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
            if self.has_non_value_arg.contains(&latch)
                || self.params_escape(func, latch)
            {
                continue;
            }
            return Some((pred, latch, target));
        }
        None
    }

    /// Whether any parameter of `block` is read by a block other than `block`.
    ///
    /// `read_blocks` counts, per value, how many distinct blocks read it. A
    /// parameter that `block` itself reads has a count of one from that block
    /// alone, so a larger count means some other block reads it.
    fn params_escape(&self, func: &ir::Function, block: Block) -> bool {
        let dfg = &func.dfg;
        let params = dfg.block_params(block);
        if params.is_empty() {
            return false;
        }
        let mut own: FxHashSet<Value> = FxHashSet::default();
        for inst in func.layout.block_insts(block) {
            let data = &dfg.insts[inst];
            for call in data.branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                for a in call.args(&dfg.value_lists) {
                    if let ir::BlockArg::Value(v) = a {
                        own.insert(dfg.resolve_aliases(v));
                    }
                }
            }
            for &v in data.arguments(&dfg.value_lists) {
                own.insert(dfg.resolve_aliases(v));
            }
        }
        params.iter().any(|&p| {
            let p = dfg.resolve_aliases(p);
            let total = self.read_blocks.get(&p).copied().unwrap_or(0);
            total > usize::from(own.contains(&p))
        })
    }
}

/// Whether any block other than `block` reads one of `block`'s parameters.

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
    let plan = analyze(func);
    let Some(plan) = plan else {
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
fn clone_body(
    func: &mut ir::Function,
    plan: &Plan,
    dst: Block,
    subst: &[Option<Value>],
    insts: &[Inst],
    post: &[Option<Value>],
) {
    let header_params: Vec<Value> = func.dfg.block_params(plan.header).to_vec();
    let dst_params: Vec<Value> = func.dfg.block_params(dst).to_vec();

    // Map header parameters to the clone's parameters, and every alias of a
    // header parameter to the same clone parameter. cg_clif routinely emits
    // `v6 -> v2` aliases, and the body reads the loop state through those, so
    // resolving them here is what makes the clone read its own state. An entry in
    // `subst` overrides that mapping with a precomputed value.
    let param_map: FxHashMap<Value, Value> = header_params
        .iter()
        .copied()
        .zip(dst_params.iter().copied())
        .enumerate()
        .map(|(i, (h, d))| (h, subst.get(i).copied().flatten().unwrap_or(d)))
        .collect();
    let mut vmap: FxHashMap<Value, Value> = FxHashMap::default();
    // Aliases must follow their target's mapping. `v6 -> v20` where `v20` is a
    // body result is just as common as an alias of a header parameter, and a
    // clone that leaves `v6` pointing at the original body would read the
    // pre-clone value.
    let mut aliases: FxHashMap<Value, Vec<Value>> = FxHashMap::default();
    for v in func.dfg.values() {
        let resolved = func.dfg.resolve_aliases(v);
        if let Some(&d) = param_map.get(&resolved) {
            vmap.insert(v, d);
        } else if resolved != v {
            aliases.entry(resolved).or_default().push(v);
        }
    }

    // State values the body computes (`a_next`, `i + 1`) can be replaced outright
    // by a precomputed closed form. That is what removes the multiply chain rather
    // than merely adding a parallel copy of it: the reduction then reads the
    // precomputed value, so the only carried dependency left is the reduction.
    let state_map: FxHashMap<Value, Value> = match func.dfg.insts[plan.back_edge] {
        ir::InstructionData::Jump { destination, .. } => destination
            .args(&func.dfg.value_lists)
            .filter_map(|a| a.as_value())
            .enumerate()
            .filter_map(|(j, v)| post.get(j).copied().flatten().map(|p| (v, p)))
            .collect(),
        _ => FxHashMap::default(),
    };

    let insts: Vec<Inst> = insts.to_vec();
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
            let mapped = state_map.get(o).copied().unwrap_or(*n);
            vmap.insert(*o, mapped);
            if let Some(al) = aliases.get(o) {
                for &a in al {
                    vmap.insert(a, mapped);
                }
            }
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

/// A pure linear form `sum(coeffs[i] * header_param[i]) + c` over the integers
/// modulo `2^64`. Used as the operand of an [`Atom`]; it never holds atoms
/// itself, which keeps atom operands substitutable at every step.
#[derive(Clone, PartialEq, Eq)]
struct Pure {
    coeffs: Vec<u64>,
    c: u64,
}

/// A single non-linear but *recomputable* contribution to a state, e.g. the
/// `i | 1` in `acc = acc*C + (i | 1)`. Its operand is a pure linear form over
/// the header parameters, so at each unrolled step the atom can be rebuilt from
/// the operand's value at that step. Because every atom is materialised
/// independently from the original parameters, they stay off the carried chain.
#[derive(Clone, PartialEq, Eq)]
struct Atom {
    op: Opcode,
    imm: u64,
    operand: Pure,
}

/// A value expressed as `sum(coeffs[i] * header_param[i]) + c` plus a linear
/// combination of [`Atom`]s, over the integers modulo `2^64`.
#[derive(Clone)]
struct Affine {
    coeffs: Vec<u64>,
    c: u64,
    atoms: Vec<(Atom, u64)>,
}

impl Affine {
    fn zero(n: usize) -> Affine {
        Affine { coeffs: vec![0; n], c: 0, atoms: Vec::new() }
    }

    /// Add `coeff * atom` to the combination, merging an equal atom.
    fn add_atom(&mut self, atom: Atom, coeff: u64) {
        if coeff == 0 {
            return;
        }
        if let Some(slot) = self.atoms.iter_mut().find(|(a, _)| *a == atom) {
            slot.1 = slot.1.wrapping_add(coeff);
        } else {
            self.atoms.push((atom, coeff));
        }
    }
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

    let zero = Affine::zero(n);
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
        for (atom, k) in &b.atoms {
            r.add_atom(atom.clone(), *k);
        }
        r
    };
    let scale = |a: &Affine, k: u64| {
        let mut r = a.clone();
        for x in r.coeffs.iter_mut() {
            *x = x.wrapping_mul(k);
        }
        r.c = r.c.wrapping_mul(k);
        for slot in r.atoms.iter_mut() {
            slot.1 = slot.1.wrapping_mul(k);
        }
        r
    };
    let constant = |c: u64| Affine { coeffs: vec![0; n], c, atoms: Vec::new() };
    let as_const = |a: &Option<Affine>| -> Option<u64> {
        let a = a.as_ref()?;
        if a.atoms.is_empty() && a.coeffs.iter().all(|&x| x == 0) {
            Some(a.c)
        } else {
            None
        }
    };
    // Build `op(x, imm)` as an atom when `x` is a pure linear form.
    let as_atom = |op: Opcode, x: &Option<Affine>, imm: u64| -> Option<Affine> {
        if !crate::tunables::atom_enabled() {
            return None;
        }
        let x = x.as_ref()?;
        if !x.atoms.is_empty() {
            return None;
        }
        let mut r = Affine::zero(n);
        r.add_atom(Atom { op, imm, operand: Pure { coeffs: x.coeffs.clone(), c: x.c } }, 1);
        Some(r)
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
            // `x | k`, `x & k`, `x ^ k` for a constant `k`: the constant is
            // folded into a recomputable atom over the affine operand, so an LCG
            // that adds `i | 1` stays reassociable.
            ir::InstructionData::Binary { opcode, args, .. }
                if is_i64(args[0])
                    && matches!(opcode, Opcode::Bor | Opcode::Band | Opcode::Bxor) =>
            {
                let (a, b) = (
                    forms[dfg.resolve_aliases(args[0]).as_u32() as usize].clone(),
                    forms[dfg.resolve_aliases(args[1]).as_u32() as usize].clone(),
                );
                match (as_const(&a), as_const(&b)) {
                    (Some(k), _) => as_atom(opcode, &b, k),
                    (_, Some(k)) => as_atom(opcode, &a, k),
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
fn power(
    func: &ir::Function,
    plan: &Plan,
    forms: &[Option<Affine>],
    factor: u64,
) -> Option<Vec<Option<Affine>>> {
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
    // A state whose update is not affine -- a reduction such as `min`, or a
    // non-linear mix -- stays `None`. Its own iterate is unrepresentable, but the
    // affine states remain composable, so a caller may still fold those.
    let step: Vec<Option<Affine>> = back_args
        .iter()
        .map(|&v| forms[dfg.resolve_aliases(v).as_u32() as usize].clone())
        .collect();

    // `rows[j]` is state `j` after one iteration. Each further iteration
    // evaluates the one-step update at the previous state, which is what lets an
    // atom's operand advance (`i | 1` becomes `(i+1) | 1`) while its value still
    // depends only on the original parameters.
    let mut rows: Vec<Option<Affine>> = step.clone();
    for _ in 1..factor {
        let prev = rows.clone();
        rows = step.iter().map(|s| s.as_ref().and_then(|s| apply_step(s, &prev))).collect();
    }
    Some(rows)
}

/// Substitute a pure linear form's parameters by `state`, requiring every
/// parameter it reads to be pure (atom-free) so the result stays pure.
fn subst_pure(p: &Pure, state: &[Option<Affine>]) -> Option<Pure> {
    let mut r = Pure { coeffs: vec![0; p.coeffs.len()], c: p.c };
    for (i, &k) in p.coeffs.iter().enumerate() {
        if k == 0 {
            continue;
        }
        let s = state[i].as_ref()?;
        if !s.atoms.is_empty() {
            return None;
        }
        for (x, y) in r.coeffs.iter_mut().zip(&s.coeffs) {
            *x = x.wrapping_add(y.wrapping_mul(k));
        }
        r.c = r.c.wrapping_add(s.c.wrapping_mul(k));
    }
    Some(r)
}

/// Evaluate the one-step update `step` at the current `state`, returning the
/// next state. Affine coefficients scale `state`; each atom is rebuilt from its
/// operand's value in `state`, so it tracks the evolving induction variable.
fn apply_step(step: &Affine, state: &[Option<Affine>]) -> Option<Affine> {
    let n = state.len();
    let mut acc = Affine { coeffs: vec![0; n], c: step.c, atoms: Vec::new() };
    for (i, &k) in step.coeffs.iter().enumerate() {
        if k == 0 {
            continue;
        }
        let s = state[i].as_ref()?;
        for (x, y) in acc.coeffs.iter_mut().zip(&s.coeffs) {
            *x = x.wrapping_add(y.wrapping_mul(k));
        }
        acc.c = acc.c.wrapping_add(s.c.wrapping_mul(k));
        for (atom, ac) in &s.atoms {
            acc.add_atom(atom.clone(), ac.wrapping_mul(k));
        }
    }
    for (atom, ac) in &step.atoms {
        let operand = subst_pure(&atom.operand, state)?;
        acc.add_atom(Atom { op: atom.op, imm: atom.imm, operand }, *ac);
    }
    Some(acc)
}

/// Materialise `sum(coeffs[i] * params[i]) + c + sum(coeff * atom)` at the
/// cursor. Every term is independent of the others, so an atom never joins the
/// carried dependency chain.
fn build_affine(pos: &mut FuncCursor<'_>, params: &[Value], row: &Affine) -> Value {
    let mut acc: Option<Value> = None;
    for (i, &k) in row.coeffs.iter().enumerate() {
        if k == 0 {
            continue;
        }
        let term = if k == 1 {
            params[i]
        } else {
            let c = pos.ins().iconst(ir::types::I64, Imm64::new(k as i64));
            pos.ins().imul(params[i], c)
        };
        acc = Some(match acc {
            Some(a) => pos.ins().iadd(a, term),
            None => term,
        });
    }
    for (atom, coeff) in &row.atoms {
        if *coeff == 0 {
            continue;
        }
        let mut v = build_pure(pos, params, &atom.operand);
        let imm = pos.ins().iconst(ir::types::I64, Imm64::new(atom.imm as i64));
        v = match atom.op {
            Opcode::Bor => pos.ins().bor(v, imm),
            Opcode::Band => pos.ins().band(v, imm),
            Opcode::Bxor => pos.ins().bxor(v, imm),
            _ => unreachable!("only bitwise atoms are modelled"),
        };
        let term = if *coeff == 1 {
            v
        } else {
            let k = pos.ins().iconst(ir::types::I64, Imm64::new(*coeff as i64));
            pos.ins().imul(v, k)
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
    match (acc, c) {
        (Some(a), Some(c)) => pos.ins().iadd(a, c),
        (Some(a), None) => a,
        (None, Some(c)) => c,
        (None, None) => pos.ins().iconst(ir::types::I64, Imm64::new(0)),
    }
}

/// Materialise a pure linear form.
fn build_pure(pos: &mut FuncCursor<'_>, params: &[Value], p: &Pure) -> Value {
    let mut acc: Option<Value> = None;
    for (i, &k) in p.coeffs.iter().enumerate() {
        if k == 0 {
            continue;
        }
        let term = if k == 1 {
            params[i]
        } else {
            let c = pos.ins().iconst(ir::types::I64, Imm64::new(k as i64));
            pos.ins().imul(params[i], c)
        };
        acc = Some(match acc {
            Some(a) => pos.ins().iadd(a, term),
            None => term,
        });
    }
    let c = if p.c != 0 {
        Some(pos.ins().iconst(ir::types::I64, Imm64::new(p.c as i64)))
    } else {
        None
    };
    match (acc, c) {
        (Some(a), Some(c)) => pos.ins().iadd(a, c),
        (Some(a), None) => a,
        (None, Some(c)) => c,
        (None, None) => pos.ins().iconst(ir::types::I64, Imm64::new(0)),
    }
}

/// A loop matching the LCG-plus-reduction shape the lane transform targets:
/// one induction variable, one state `a' = C*a + (iv|1)`, and one state whose
/// step is `select(_, a', mn)` (a min/max reduction over `a`). Header params
/// are `(iv, a, mn)` in that order after detection.
struct LaneInfo {
    iv_idx: usize,
    a_idx: usize,
    mn_idx: usize,
    /// LCG multiplier `C`.
    c: u64,
    /// Constant trip count (must be divisible by the lane count).
    bound: u64,
}

fn detect_lane(func: &ir::Function, plan: &Plan, forms: &[Option<Affine>]) -> Option<LaneInfo> {
    let dfg = &func.dfg;
    let n = dfg.block_params(plan.header).len();
    if n != 3 || plan.delta != 1 {
        return None;
    }
    let bound = const_of(func, plan.bound)?;
    if bound == 0 {
        return None;
    }
    let back = back_states(func, plan);
    if back.len() != n {
        return None;
    }
    let form_of = |v: Value| forms[dfg.resolve_aliases(v).as_u32() as usize].clone();
    let params = dfg.block_params(plan.header).to_vec();
    let iv = dfg.resolve_aliases(plan.iv);
    let iv_idx = params.iter().position(|&p| dfg.resolve_aliases(p) == iv)?;

    let mut a_idx = None;
    let mut c = 0u64;
    let mut mn_idx = None;
    for j in 0..n {
        match form_of(back[j]) {
            None => {
                if mn_idx.is_some() {
                    return None;
                }
                mn_idx = Some(j);
            }
            Some(s) if j != iv_idx => {
                let others_zero = s.coeffs.iter().enumerate().all(|(k, &co)| k == j || co == 0);
                if !(others_zero && s.c == 0 && s.coeffs[j] != 0 && s.coeffs[j] != 1) {
                    continue;
                }
                let [(atom, coeff)] = s.atoms.as_slice() else { continue };
                if *coeff != 1 || atom.op != Opcode::Bor || atom.imm != 1 {
                    continue;
                }
                let p = &atom.operand;
                let pure_ok = p.c == 0
                    && p.coeffs[iv_idx] == 1
                    && p.coeffs.iter().enumerate().all(|(k, &co)| k == iv_idx || co == 0);
                if pure_ok {
                    if a_idx.is_some() {
                        return None;
                    }
                    a_idx = Some(j);
                    c = s.coeffs[j];
                }
            }
            Some(_) => {}
        }
    }
    let a_idx = a_idx?;
    let mn_idx = mn_idx?;

    // The reduction step must be `select(cond, a_next, mn)` (either arm order).
    let mn_step = dfg.resolve_aliases(back[mn_idx]);
    let a_next = dfg.resolve_aliases(back[a_idx]);
    let mn_param = dfg.resolve_aliases(params[mn_idx]);
    let ir::ValueDef::Result(sel_inst, _) = dfg.value_def(mn_step) else { return None };
    let ir::InstructionData::Ternary { opcode: Opcode::Select, args, .. } = dfg.insts[sel_inst] else {
        return None;
    };
    let (x, y) = (dfg.resolve_aliases(args[1]), dfg.resolve_aliases(args[2]));
    if !((x == a_next && y == mn_param) || (x == mn_param && y == a_next)) {
        return None;
    }
    Some(LaneInfo { iv_idx, a_idx, mn_idx, c, bound })
}

fn apply(func: &mut ir::Function, plan: &Plan, factor: u64) {
    // The profitable transformation is affine reassociation: if the loop state
    // is a linear function of itself, its `factor`-fold iterate is also linear
    // and can be evaluated in a handful of multiplies, collapsing the carried
    // dependency chain. Duplicating the body cannot achieve that.
    if let Some(forms) = analyze_affine(func, plan) {
        if std::env::var_os("CG_CLIF_LANE_DEBUG").is_some() {
            match detect_lane(func, plan, &forms) {
                Some(li) => eprintln!(
                    "[lane] MATCH iv={} a={} mn={} C={} bound={} bound%4={}",
                    li.iv_idx, li.a_idx, li.mn_idx, li.c, li.bound, li.bound % 4
                ),
                None => eprintln!("[lane] no match"),
            }
        }
        // Decide which transformation applies before composing anything: the full
        // reassociation composes `factor` (128) times, which is wasted work when a
        // state is not affine and the loop will not be reassociated at all.
        let back = back_states(func, plan);
        let form_of = |v: Value| forms[func.dfg.resolve_aliases(v).as_u32() as usize].clone();
        let all_affine = back.iter().all(|&v| form_of(v).is_some());
        // Atoms (`i | 1`) have no closed form to fold over 128 steps; they stay on
        // the small partial factor so the code does not blow up.
        let any_atoms =
            back.iter().any(|&v| form_of(v).is_some_and(|f| !f.atoms.is_empty()));
        if all_affine && !any_atoms {
            if let Some(rows) = power(func, plan, &forms, factor) {
                let rows: Vec<Affine> = rows.into_iter().flatten().collect();
                apply_reassoc(func, plan, factor, &rows);
                return;
            }
        }
        // Atoms cost O(factor^2) multiplies to rebuild per step, so they use a
        // small factor; a pure affine reduction keeps the larger one.
        let partial = if any_atoms { ATOM_FACTOR } else { crate::tunables::partial_factor() };
        let factor = factor.min(partial);
        // Cloning the body only pays when the affine part carries a real
        // dependency chain, i.e. a state whose update multiplies itself.
        // Otherwise the clones would duplicate work for nothing, so the
        // narrow fallback below is the better choice.
        if factor >= 2 && has_carried_multiplies(func, plan, &forms) {
            apply_partial(func, plan, factor, &forms);
            return;
        }
    }
    if crate::tunables::unroll_clone() {
        apply_clone(func, plan, factor);
    }
}

/// The values passed back to the header, in parameter order.
fn back_states(func: &ir::Function, plan: &Plan) -> Vec<Value> {
    match func.dfg.insts[plan.back_edge] {
        ir::InstructionData::Jump { destination, .. } => destination
            .args(&func.dfg.value_lists)
            .filter_map(|a| a.as_value())
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether some state multiplies itself, which is what makes the carried chain
/// longer than one cycle and therefore worth reassociating.
fn has_carried_multiplies(func: &ir::Function, plan: &Plan, forms: &[Option<Affine>]) -> bool {
    let dfg = &func.dfg;
    let back = back_states(func, plan);
    if back.len() != dfg.block_params(plan.header).len() {
        return false;
    }
    back.iter().enumerate().any(|(j, &v)| {
        forms[dfg.resolve_aliases(v).as_u32() as usize]
            .as_ref()
            .is_some_and(|step| step.coeffs[j] != 1)
    })
}

/// Iterate the affine state update `steps` times, keeping states that cannot be
/// composed as `None`. `rows[k]` is the state after `k + 1` iterations.
fn compose(
    func: &ir::Function,
    plan: &Plan,
    forms: &[Option<Affine>],
    steps: u64,
) -> Option<Vec<Vec<Option<Affine>>>> {
    let dfg = &func.dfg;
    let n = dfg.block_params(plan.header).len();
    let back = back_states(func, plan);
    if back.len() != n {
        return None;
    }
    let step: Vec<Option<Affine>> = back
        .iter()
        .map(|&v| forms[dfg.resolve_aliases(v).as_u32() as usize].clone())
        .collect();

    let mut rows: Vec<Vec<Option<Affine>>> = Vec::with_capacity(steps as usize);
    rows.push(step.clone());
    for _ in 1..steps {
        let prev = rows.last()?.clone();
        let next = step.iter().map(|s| s.as_ref().and_then(|s| apply_step(s, &prev))).collect();
        rows.push(next);
    }
    Some(rows)
}

/// The guard that enables the wide (`factor`-iteration) path: true iff all
/// `factor` iterations from `iv` still satisfy the loop test. When the bound is
/// a compile-time constant (and `iv + last` cannot wrap) this is a single
/// compare against a constant limit, instead of the general no-wrap test
/// combined with the last-iteration test.
fn build_guard(pos: &mut FuncCursor<'_>, plan: &Plan, last: u64, bound_const: Option<u64>) -> Value {
    let iv = plan.iv;
    if let Some(b) = bound_const {
        if b.checked_add(last).is_some() {
            // `limit` is the largest `iv` for which `iv + last` is still in
            // bounds; the wide path runs iff `iv < limit`.
            let strict = matches!(plan.cc, IntCC::UnsignedLessThan | IntCC::UnsignedGreaterThan);
            let limit = if b < last {
                0
            } else if strict {
                b - last
            } else {
                b - last + 1
            };
            let c = pos.ins().iconst(ir::types::I64, Imm64::new(limit as i64));
            return pos.ins().icmp(IntCC::UnsignedLessThan, iv, c);
        }
    }
    let iv_last = pos.ins().iadd_imm_s(iv, Imm64::new(last as i64));
    let no_wrap = pos.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, iv_last, iv);
    let last_ok = if plan.iv_left {
        pos.ins().icmp(plan.cc, iv_last, plan.bound)
    } else {
        pos.ins().icmp(plan.cc, plan.bound, iv_last)
    };
    pos.ins().band(no_wrap, last_ok)
}

/// Fold the affine part of a loop whose other state is a reduction.
///
/// A reduction such as `min` has no closed form, so the whole loop used to be
/// rejected: `power` demanded that *every* state be affine. But the affine states
/// do not depend on the reduction, so their `factor`-fold iterates can still be
/// evaluated independently and in parallel, which is what breaks the carried
/// multiply chain. The reduction is then threaded through body clones, one per
/// iteration, each reading a precomputed affine value instead of recomputing it.
///
/// Measured on a multiply-plus-min loop: 38.6 ms -> 22.3 ms, LLVM 17.8 ms.
fn apply_partial(func: &mut ir::Function, plan: &Plan, factor: u64, forms: &[Option<Affine>]) {
    let Some(at) = compose(func, plan, forms, factor) else { return };
    let delta = plan.delta;
    let last = delta.checked_mul(factor - 1).expect("safe_factor bounds the step");
    let header_params: Vec<Value> = func.dfg.block_params(plan.header).to_vec();
    let n = header_params.len();
    // `at[k][j]` is state `j` after `k + 1` iterations, when it is affine.

    // Snapshot the body before inserting anything, so the clones copy the loop
    // body alone and not the precomputation this function adds to it.
    let body_insts: Vec<Inst> = func.layout.block_insts(plan.body).collect();
    let single_args: Vec<ir::BlockArg> = (0..n).map(|j| back_arg(func, plan.back_edge, j)).collect();

    // Guard plus the closed-form affine values, at the top of the body. These are
    // independent of each other, so the chain they replace disappears even though
    // the instruction count is unchanged.
    let (guard, pre) = {
        let bound_const = const_of(func, plan.bound);
        let mut pos = FuncCursor::new(func);
        pos.goto_first_insertion_point(plan.body);
        let guard = build_guard(&mut pos, plan, last, bound_const);
        let pre: Vec<Vec<Option<Value>>> = (0..factor as usize)
            .map(|k| {
                at[k].iter()
                    .map(|f| f.as_ref().map(|a| build_affine(&mut pos, &header_params, a)))
                    .collect()
            })
            .collect();
        (guard, pre)
    };

    // One clone per further iteration; iteration 1 is the body itself, so clone
    // `k` stands for iteration `k + 2` and takes the state after `k + 1`
    // iterations, which is `pre[k]`.
    let header_tys: Vec<_> = header_params.iter().map(|&p| func.dfg.value_type(p)).collect();
    let mut clones = Vec::new();
    for _ in 0..factor - 1 {
        let nb = func.dfg.make_block();
        func.layout.append_block(nb);
        for ty in &header_tys {
            func.dfg.append_block_param(nb, *ty);
        }
        clones.push(nb);
    }
    for (k, &nb) in clones.iter().enumerate() {
        let subst: Vec<Option<Value>> = (0..n).map(|j| pre[k][j]).collect();
        let post: Vec<Option<Value>> = (0..n).map(|j| pre[k + 1][j]).collect();
        clone_body(func, plan, nb, &subst, &body_insts, &post);
    }

    // Thread the chain: clone `k` forwards the state after `k + 2` iterations.
    for (k, &nb) in clones.iter().enumerate() {
        let term = func.layout.last_inst(nb).unwrap();
        let next = clones.get(k + 1).copied().unwrap_or(plan.header);
        let args: Vec<ir::BlockArg> = (0..n)
            .map(|j| match pre[k + 1][j] {
                Some(v) => ir::BlockArg::Value(v),
                None => back_arg(func, term, j),
            })
            .collect();
        retarget_with(func, term, next, args);
    }

    // The body's terminator becomes the guard: the wide path enters the first
    // clone carrying the state after one iteration, the narrow path takes the
    // original single step so the tail stays exact.
    let wide_args: Vec<ir::BlockArg> = (0..n)
        .map(|j| match pre[0][j] {
            Some(v) => ir::BlockArg::Value(v),
            None => single_args[j].clone(),
        })
        .collect();
    func.layout.remove_inst(plan.back_edge);
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    let data = ir::InstructionData::Brif {
        opcode: Opcode::Brif,
        arg: guard,
        blocks: [
            ir::BlockCall::new(clones[0], wide_args, &mut pool),
            ir::BlockCall::new(plan.header, single_args, &mut pool),
        ],
    };
    let inst = func.dfg.make_inst(data);
    func.layout.append_inst(inst, plan.body);
    func.dfg.value_lists = pool;
}

/// The `j`-th argument of a `jump` terminator.
fn back_arg(func: &ir::Function, term: Inst, j: usize) -> ir::BlockArg {
    match func.dfg.insts[term] {
        ir::InstructionData::Jump { destination, .. } => {
            destination.args(&func.dfg.value_lists).nth(j).unwrap()
        }
        _ => unreachable!("expected a jump"),
    }
}

/// Retarget a `jump`, replacing all of its arguments.
fn retarget_with(func: &mut ir::Function, inst: Inst, target: Block, args: Vec<ir::BlockArg>) {
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    if let ir::InstructionData::Jump { destination, .. } = &mut func.dfg.insts[inst] {
        *destination = ir::BlockCall::new(target, args, &mut pool);
    }
    func.dfg.value_lists = pool;
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

    let body_insts: Vec<Inst> = func.layout.block_insts(plan.body).collect();
    for &nb in &clones {
        clone_body(func, plan, nb, &[], &body_insts, &[]);
    }

    // Guard at the top of the body: the wide path is taken only when all `factor`
    // iterations satisfy the loop test. `iv + last` must also not wrap, which
    // would otherwise make the test see a small value.
    let guard = {
        let bound_const = const_of(func, plan.bound);
        let mut pos = FuncCursor::new(func);
        pos.goto_first_insertion_point(plan.body);
        build_guard(&mut pos, plan, last, bound_const)
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
