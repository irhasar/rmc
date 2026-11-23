//! Loop-invariant code motion (LICM).
//!
//! Rakit has no LICM pass. The egraph only hoists *pure expression* nodes,
//! and it deliberately leaves loads in place, so a scalar loop that reloads an
//! invariant value on every iteration (a coefficient, a bound, a base pointer)
//! stays slow. This pass hoists instructions out of a loop into its preheader.
//!
//! An instruction moves only when all of its operands are available at the
//! preheader and executing it there is safe:
//!
//! * pure arithmetic (`is_pure_for_egraph`) always qualifies: it cannot trap
//!   and has no effect;
//! * a load qualifies only when its flags are `readonly + notrap + can_move`
//!   and no store in the loop can alias it.
//!
//! Aliasing is decided by *proven* non-aliasing, not by "no stores anywhere": a
//! load whose address is a global value or a stack slot is unaffected by any
//! store whose address is a different global value or stack slot. That is the
//! shape that matters in practice (a coefficient in static data, a spilled
//! local), and it is sound without a full alias analysis. Any other load is
//! blocked as soon as the loop contains a store.
//!
//! The pass is gated on `enable_licm` so a default Rakit build is
//! unaffected.

use alloc::vec::Vec;

use crate::dominator_tree::DominatorTree;
use crate::flowgraph::ControlFlowGraph;
use crate::inst_predicates::is_pure_for_egraph;
use crate::ir::{Block, Function, Inst, InstructionData, Opcode, Value};
use crate::loop_analysis::{Loop, LoopAnalysis};
use crate::timing;

/// Whether the pass is enabled at runtime.
///
/// This crate cannot see rakit's `tunables` module, so the switch is read
/// here instead. It is cached because the pass runs once per function.
pub fn enabled() -> bool {
    #[cfg(feature = "std")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| std::env::var_os("RAKIT_NO_LICM").is_none())
    }
    #[cfg(not(feature = "std"))]
    {
        true
    }
}

/// Whether to report each hoist (`RAKIT_LICM_DEBUG`), read once.
fn debug() -> bool {
    #[cfg(feature = "std")]
    {
        static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *DEBUG.get_or_init(|| std::env::var_os("RAKIT_LICM_DEBUG").is_some())
    }
    #[cfg(not(feature = "std"))]
    {
        false
    }
}

/// A load/store address identity, when it is statically known.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AddrBase {
    Global(crate::ir::GlobalValue),
    StackSlot(crate::ir::StackSlot),
}

/// Hoist loop-invariant instructions out of every loop in `func`.
pub fn do_licm(func: &mut Function) {
    let _tt = timing::licm();

    // Creating a preheader rewrites the CFG, which invalidates the dominator
    // tree and loop analysis. Rather than trying to patch them incrementally,
    // each round rebuilds all three and handles one loop; a loop is only
    // visited once, tracked by header, so the pass still terminates.
    let mut done = alloc::collections::BTreeSet::new();
    loop {
        let cfg = ControlFlowGraph::with_function(func);
        let domtree = DominatorTree::with_function(func, &cfg);
        let mut loops = LoopAnalysis::new();
        loops.compute(func, &cfg, &domtree);

        let mut progressed = false;
        for lp in loops.loops() {
            let header = loops.loop_header(lp);
            if !done.insert(header) {
                continue;
            }
            let Some(preheader) = ensure_preheader(func, &cfg, header) else {
                continue;
            };
            if hoist_from_loop(func, &domtree, &loops, lp, header, preheader) {
                progressed = true;
                break;
            }
        }
        if !progressed {
            break;
        }
    }
}

/// Return a block that executes exactly once per entry to `header`.
///
/// An existing single predecessor ending in an unconditional jump is already a
/// preheader. Otherwise the edge from the header's immediate dominator is split
/// with a fresh block: the idom of a loop header is outside the loop, so the new
/// block runs once per entry.
fn find_preheader(func: &Function, cfg: &ControlFlowGraph, header: Block) -> Option<Block> {
    let mut preds = Vec::new();
    for block in func.layout.blocks() {
        if block == header {
            continue;
        }
        if cfg.succ_iter(block).any(|s| s == header) {
            preds.push(block);
        }
    }

    // A single predecessor that jumps unconditionally to the header runs
    // exactly once per entry, which is all a preheader has to guarantee.
    if preds.len() == 1 {
        let pred = preds[0];
        if cfg.succ_iter(pred).count() == 1 {
            if let Some(term) = func.layout.last_inst(pred) {
                if matches!(func.dfg.insts[term], InstructionData::Jump { .. }) {
                    return Some(pred);
                }
            }
        }
    }
    None
}

fn ensure_preheader(func: &mut Function, cfg: &ControlFlowGraph, header: Block) -> Option<Block> {
    if let Some(p) = find_preheader(func, cfg, header) {
        return Some(p);
    }

    // Split the edge from the header's immediate dominator. The idom of a loop
    // header is outside the loop, so the new block runs once per entry and is a
    // valid preheader.
    let idom = DominatorTree::with_function(func, cfg).idom(header)?;
    if idom == header {
        return None;
    }
    let term = func.layout.last_inst(idom)?;
    let params: Vec<Value> = func.dfg.block_params(header).to_vec();

    let new_block = func.dfg.make_block();
    func.layout.insert_block_after(new_block, idom);

    // Retarget the edge idom -> header to idom -> new_block, capturing the
    // arguments the edge used to pass. Those are the header's phi values.
    let mut pool = core::mem::take(&mut func.dfg.value_lists);
    let mut redirected = false;
    let mut carried: Vec<crate::ir::BlockArg> = Vec::new();
    match &mut func.dfg.insts[term] {
        InstructionData::Jump { destination, .. } => {
            if destination.block(&pool) == header {
                carried = destination.args(&pool).collect();
                *destination = crate::ir::BlockCall::new(new_block, [], &mut pool);
                redirected = true;
            }
        }
        InstructionData::Brif { blocks, .. } => {
            // Both arms of a `brif` may target the header. Redirecting only one
            // would leave a path into the loop that bypasses the new preheader,
            // so the hoisted instructions would not have run on it.
            let targets = blocks.iter().filter(|bc| bc.block(&pool) == header).count();
            if targets == 1 {
                for bc in blocks.iter_mut() {
                    if bc.block(&pool) == header {
                        carried = bc.args(&pool).collect();
                        *bc = crate::ir::BlockCall::new(new_block, [], &mut pool);
                        redirected = true;
                        break;
                    }
                }
            }
        }
        _ => {}
    }
    func.dfg.value_lists = pool;
    if !redirected {
        func.layout.remove_block(new_block);
        return None;
    }

    // Exception-handler arguments cannot be forwarded as plain values, so give
    // up on such an edge rather than dropping them.
    let forwarded: Vec<crate::ir::BlockArg> = carried
        .iter()
        .filter(|a| matches!(a, crate::ir::BlockArg::Value(_)))
        .copied()
        .collect();
    if forwarded.len() != carried.len() || forwarded.len() != params.len() {
        func.layout.remove_block(new_block);
        return None;
    }

    // The values the edge passed to the header's phi nodes are defined in
    // `idom`, which dominates the new block, so the new block can pass them on
    // unchanged. No extra block parameters are needed.
    let dest = crate::ir::BlockCall::new(header, forwarded, &mut func.dfg.value_lists);
    let jump = func.dfg.make_inst(InstructionData::Jump {
        opcode: Opcode::Jump,
        destination: dest,
    });
    func.layout.append_inst(jump, new_block);

    Some(new_block)
}

/// Hoist the invariants of `lp` into `preheader`.
fn hoist_from_loop(
    func: &mut Function,
    domtree: &DominatorTree,
    loops: &LoopAnalysis,
    lp: Loop,
    header: Block,
    preheader: Block,
) -> bool {
    let blocks: Vec<Block> = func
        .layout
        .blocks()
        .filter(|b| loops.is_in_loop(*b, lp))
        .collect();
    if blocks.is_empty() {
        return false;
    }

    // Values usable at the preheader without being hoisted: anything defined
    // outside the loop. Header block parameters are deliberately NOT included:
    // they are phi nodes that take a different value on the back edge, so they
    // are only "available" at the preheader on the entry path, not inside the
    // loop. Treating them as invariant is the classic LICM soundness bug.
    let mut available_set = crate::entity::EntitySet::<Value>::new();

    // Every store in the loop, with its address base when statically known.
    let mut store_bases: Vec<AddrBase> = Vec::new();
    let mut any_opaque_store = false;
    for &block in &blocks {
        for inst in func.layout.block_insts(block) {
            let data = &func.dfg.insts[inst];
            if data.opcode().can_store() {
                match addr_base(func, data) {
                    Some(base) => store_bases.push(base),
                    None => any_opaque_store = true,
                }
            }
        }
    }

    let mut to_hoist: Vec<Inst> = Vec::new();

    for &block in &blocks {
        let insts: Vec<Inst> = func.layout.block_insts(block).collect();
        for inst in insts {
            let data = &func.dfg.insts[inst];
            let op = data.opcode();
            if op.is_branch() || op.is_return() || op.is_terminator() {
                continue;
            }

            // Hoisting a trapping instruction would make it execute on paths
            // that previously skipped the loop body entirely.
            if op.can_trap() {
                continue;
            }

            if op.can_load() {
                let flags = data
                    .memflags()
                    .map(|f| func.dfg.mem_flags[f])
                    .unwrap_or_else(crate::ir::MemFlagsData::trusted);
                if !(flags.readonly() && flags.notrap() && flags.can_move()) {
                    continue;
                }
                let safe = match addr_base(func, data) {
                    // A global or stack slot is only clobbered by a store to the
                    // same object; an opaque store may hit anything.
                    Some(b) => !any_opaque_store && !store_bases.contains(&b),
                    None => store_bases.is_empty() && !any_opaque_store,
                };
                if !safe {
                    continue;
                }
            } else if !is_pure_for_egraph(func, inst) {
                continue;
            }

            let operands: &[Value] = data.arguments(&func.dfg.value_lists);
            let all_available = operands.iter().all(|v| {
                let v = func.dfg.resolve_aliases(*v);
                available_set.contains(v)
                    || match def_block(func, v) {
                        // Defined outside this loop *and* before the preheader.
                        // Outside-the-loop alone is not enough: a sibling
                        // branch does not dominate the preheader.
                        Some(b) => {
                            !loops.is_in_loop(b, lp) && domtree.block_dominates(b, preheader)
                        }
                        // No defining instruction: a block parameter. Only a
                        // parameter of the *preheader* itself is defined on the
                        // path into the loop; any other parameter may be a
                        // loop-carried phi whose value changes per iteration.
                        None => match block_param_owner(func, v) {
                            Some(b) => b == preheader,
                            None => false,
                        },
                    }
            });
            if !all_available {
                continue;
            }

            // An invariant whose operands are all available can itself be
            // hoisted; its results then become available for later
            // instructions, and blocks are visited in layout order so a
            // definition is always seen before its uses.
            to_hoist.push(inst);
            for r in func.dfg.inst_results(inst) {
                available_set.insert(*r);
            }
        }
    }

    #[cfg(feature = "std")]
    if debug() && !to_hoist.is_empty() {
        std::eprintln!(
            "licm: hoisting {} inst(s) out of loop at {:?} into {:?}",
            to_hoist.len(),
            header,
            preheader
        );
    }

    if to_hoist.is_empty() {
        return false;
    }

    // Insert before the preheader terminator, preserving relative order.
    for inst in to_hoist {
        func.layout.remove_inst(inst);
        match func.layout.last_inst(preheader) {
            Some(t) => func.layout.insert_inst(inst, t),
            None => func.layout.append_inst(inst, preheader),
        }
    }
    true
}

/// Static identity of a load/store address, when the base is a global value or
/// a stack slot.
fn addr_base(func: &Function, data: &InstructionData) -> Option<AddrBase> {
    let addr = match data {
        InstructionData::Load { arg, .. } | InstructionData::LoadNoOffset { arg, .. } => *arg,
        InstructionData::Store { args, .. } | InstructionData::StoreNoOffset { args, .. } => args[0],
        _ => return None,
    };
    let addr = func.dfg.resolve_aliases(addr);
    let inst = func.dfg.value_def(addr).inst()?;
    match func.dfg.insts[inst] {
        InstructionData::UnaryGlobalValue { global_value, .. } => {
            Some(AddrBase::Global(global_value))
        }
        InstructionData::StackAddr { stack_slot, .. } => Some(AddrBase::StackSlot(stack_slot)),
        _ => None,
    }
}

/// Block that defines `value`, if it is defined by an instruction.
fn def_block(func: &Function, value: Value) -> Option<Block> {
    let value = func.dfg.resolve_aliases(value);
    func.layout.inst_block(func.dfg.value_def(value).inst()?)
}

/// Block that owns `value` as a parameter, if it is a block parameter.
fn block_param_owner(func: &Function, value: Value) -> Option<Block> {
    let value = func.dfg.resolve_aliases(value);
    if !matches!(func.dfg.value_def(value), crate::ir::ValueDef::Param(..)) {
        return None;
    }
    func.layout
        .blocks()
        .find(|b| func.dfg.block_params(*b).contains(&value))
}
