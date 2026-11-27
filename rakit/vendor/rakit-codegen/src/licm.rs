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
//! The pass is on by default; `RAKIT_NO_LICM=1` disables it (see `enabled`).

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

    // A slot read here is only exempt from `readonly`/`can_move` if no call
    // anywhere in the function can have kept its address, so this is a
    // function-wide fact computed once, before any hoisting moves a definition.
    let addresses = address_info(func);

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

        // Collect first: the iterator borrows `loops`, which the body would
        // also have to hand out for hoisting.
        let headers: Vec<Block> = loops.loops().map(|l| loops.loop_header(l)).collect();
        let mut progressed = false;
        for header in headers {
            if !done.insert(header) {
                continue;
            }
            let Some((preheader, created)) = ensure_preheader(func, &cfg, header) else {
                continue;
            };
            if created {
                // The analyses above predate the new block, and `func` no
                // longer matches them: rebuild, hoist into the fresh preheader,
                // then restart the round so no later header is looked up in a
                // stale CFG (an inserted block is outside its block range).
                let cfg = ControlFlowGraph::with_function(func);
                let domtree = DominatorTree::with_function(func, &cfg);
                let mut loops = LoopAnalysis::new();
                loops.compute(func, &cfg, &domtree);
                if let Some(lp) = loops.loops().find(|l| loops.loop_header(*l) == header) {
                    hoist_from_loop(func, &addresses, &domtree, &loops, lp, header, preheader);
                }
                progressed = true;
                break;
            }
            let Some(lp) = loops.loops().find(|l| loops.loop_header(*l) == header) else {
                continue;
            };
            if hoist_from_loop(func, &addresses, &domtree, &loops, lp, header, preheader) {
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

/// Return a block that executes exactly once per entry to `header`.
///
/// An existing single predecessor ending in an unconditional jump is already a
/// preheader. Otherwise a new block is inserted after the header's immediate
/// dominator and *every* edge from outside the loop is redirected through it,
/// carrying its arguments into the new block's parameters.
///
/// Redirecting only the idom edge is not enough: a second outside predecessor
/// would keep jumping straight to the header, and an instruction hoisted into
/// the preheader would not have run on that path.
///
/// Returns the preheader and whether it had to be created. Every edge is
/// validated before any of them is rewritten: a split jump table or an
/// exception edge anywhere among the entries abandons the whole attempt with
/// the CFG untouched.
fn ensure_preheader(func: &mut Function, cfg: &ControlFlowGraph, header: Block) -> Option<(Block, bool)> {
    if let Some(p) = find_preheader(func, cfg, header) {
        return Some((p, false));
    }

    let domtree = DominatorTree::with_function(func, cfg);
    let idom = domtree.idom(header)?;
    if idom == header {
        return None;
    }

    let outside: Vec<Block> = func
        .layout
        .blocks()
        .filter(|b| *b != header)
        .filter(|b| cfg.succ_iter(*b).any(|s| s == header))
        .filter(|b| !domtree.block_dominates(header, *b))
        .collect();
    if outside.is_empty() {
        return None;
    }

    let params: Vec<Value> = func.dfg.block_params(header).to_vec();

    // Pass 1: collect the edges to rewrite, without touching anything.
    // `None` as the arm index means the predecessor's only edge is an
    // unconditional jump.
    let mut edges: Vec<(Inst, Option<usize>, Vec<crate::ir::BlockArg>)> = Vec::new();
    for pred in &outside {
        let term = func.layout.last_inst(*pred)?;
        let block_call = |bc: &crate::ir::BlockCall| -> Option<Vec<crate::ir::BlockArg>> {
            if bc.block(&func.dfg.value_lists) != header {
                return None;
            }
            let carried: Vec<crate::ir::BlockArg> = bc.args(&func.dfg.value_lists).collect();
            // Exception-handler arguments cannot be forwarded as plain values.
            let plain = carried
                .iter()
                .all(|a| matches!(a, crate::ir::BlockArg::Value(_)));
            (plain && carried.len() == params.len()).then_some(carried)
        };
        match &func.dfg.insts[term] {
            InstructionData::Jump { destination, .. } => {
                edges.push((term, None, block_call(destination)?));
            }
            InstructionData::Brif { blocks, .. } => {
                let mut arms = 0usize;
                for (i, bc) in blocks.iter().enumerate() {
                    match block_call(bc) {
                        Some(carried) => {
                            edges.push((term, Some(i), carried));
                            arms += 1;
                        }
                        None if bc.block(&func.dfg.value_lists) == header => return None,
                        None => {}
                    }
                }
                if arms == 0 {
                    return None;
                }
            }
            // Anything else (a jump table, an exception edge, ...) makes the
            // loop unreachable from this entry without rewriting more than a
            // block call, so leave the function alone.
            _ => return None,
        }
    }
    if edges.is_empty() {
        return None;
    }

    // Pass 2: create the preheader and rewrite the collected edges.
    let new_block = func.dfg.make_block();
    func.layout.insert_block_after(new_block, idom);
    // Each entry edge passes its own values to the header's phi nodes, so the
    // preheader holds them in parameters of the same types and forwards them.
    let new_params: Vec<Value> = params
        .iter()
        .map(|p| {
            let ty = func.dfg.value_type(*p);
            func.dfg.append_block_param(new_block, ty)
        })
        .collect();

    let mut pool = core::mem::take(&mut func.dfg.value_lists);
    for (inst, arm, carried) in edges {
        match arm {
            None => match &mut func.dfg.insts[inst] {
                InstructionData::Jump { destination, .. } => {
                    *destination = crate::ir::BlockCall::new(new_block, carried, &mut pool);
                }
                _ => debug_assert!(false, "validated a jump, rewrote something else"),
            },
            Some(i) => match &mut func.dfg.insts[inst] {
                InstructionData::Brif { blocks, .. } => {
                    blocks[i] = crate::ir::BlockCall::new(new_block, carried, &mut pool);
                }
                _ => debug_assert!(false, "validated an arm, rewrote something else"),
            },
        }
    }
    func.dfg.value_lists = pool;

    let dest = crate::ir::BlockCall::new(
        header,
        new_params
            .iter()
            .map(|v| crate::ir::BlockArg::Value(*v))
            .collect::<Vec<_>>(),
        &mut func.dfg.value_lists,
    );
    let jump = func.dfg.make_inst(InstructionData::Jump {
        opcode: Opcode::Jump,
        destination: dest,
    });
    func.layout.append_inst(jump, new_block);

    Some((new_block, true))
}

/// Hoist the invariants of `lp` into `preheader`.
fn hoist_from_loop(
    func: &mut Function,
    addresses: &AddressInfo,
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
                    None => {
                        any_opaque_store = true;
                        // The address is not a plain `stack_addr`, but it may
                        // still be a derived one (`addr + 8`) or arrive
                        // through a block parameter. Anything it may point
                        // into is written by this store.
                        if let Some(addr) = store_address(data) {
                            let addr = func.dfg.resolve_aliases(addr);
                            if let Some(slots) = addresses.points_into.get(&addr) {
                                for &slot in slots {
                                    store_bases.push(AddrBase::StackSlot(slot));
                                }
                            }
                        }
                    }
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

            // `can_load()` also covers `atomic_load`, which is a
            // sequentially-consistent operation: hoisting it would drop the
            // ordering the loop was written for, not just move a read. Only an
            // ordinary `load` is a candidate.
            if op.can_load() && op == Opcode::Load {
                let flags = data
                    .memflags()
                    .map(|f| func.dfg.mem_flags[f])
                    .unwrap_or_else(crate::ir::MemFlagsData::trusted);
                // A trapping load is not safe to speculate; rakit also uses a
                // trapping load for `volatile_load`, which must not move.
                if !flags.notrap() {
                    continue;
                }
                let base = addr_base(func, data);
                let safe = match base {
                    // A stack slot whose address never escapes the function is
                    // reachable only through the addresses this function
                    // computes, so neither a call nor an indirect store can
                    // write it and `readonly` is not needed: a store naming the
                    // slot is the only thing that can change the value, and
                    // that is checked below.
                    Some(AddrBase::StackSlot(s)) if !addresses.escapes.contains(&s) => {
                        !store_bases.contains(&AddrBase::StackSlot(s))
                    }
                    // A global or stack slot is only clobbered by a store to the
                    // same object; an opaque store may hit anything.
                    Some(b) => {
                        flags.readonly()
                            && flags.can_move()
                            && !any_opaque_store
                            && !store_bases.contains(&b)
                    }
                    None => {
                        flags.readonly()
                            && flags.can_move()
                            && store_bases.is_empty()
                            && !any_opaque_store
                    }
                };
                if !safe {
                    continue;
                }
            } else if !is_pure_for_egraph(func, inst)
                // `stack_addr` is flagged as having side effects because it
                // reads the frame layout, but the address it yields is the same
                // everywhere in the function, so hoisting it is safe. A load
                // through such an address can only be hoisted once the address
                // is available at the preheader, so this unblocks those loads.
                && !matches!(data, InstructionData::StackAddr { .. })
            {
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
        // `store` operand order is `x` (value) then `p` (address); the address
        // is `args[1]` (see `formats.store` / `store_no_offset` in
        // `rakit-codegen-meta/src/shared/instructions.rs`). `args[0]` is the
        // stored value, which is not the address.
        InstructionData::Store { args, .. } | InstructionData::StoreNoOffset { args, .. } => {
            args[1]
        }
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

/// The address operand of a store, whatever its shape.
fn store_address(data: &InstructionData) -> Option<Value> {
    match data {
        InstructionData::Store { args, .. } | InstructionData::StoreNoOffset { args, .. } => {
            Some(args[1])
        }
        _ => None,
    }
}

/// Stack slots whose address escapes anywhere in `func`.
///
/// An address escapes when it reaches something that can keep it: a call
/// argument, a value stored to memory, a return value, or (through address
/// arithmetic) another one of those. A slot whose address never escapes can
/// only be read and written through the addresses this function computes for
/// it, so no call -- in the loop or before it -- can retain the address and
/// write the slot later, and a load from it needs no `readonly` flag to be
/// hoisted.
///
/// Addresses are followed through address arithmetic and through block
/// parameters (rakit threads stack addresses through phis constantly); a
/// branch argument is not an escape by itself, only where the value ends up.
fn address_info(func: &Function) -> AddressInfo {
    type Slots = alloc::collections::BTreeSet<crate::ir::StackSlot>;
    let mut escapes = Slots::new();
    // Values that are a stack address, and the slots they may point into.
    let mut points_into: alloc::collections::BTreeMap<Value, Slots> =
        alloc::collections::BTreeMap::new();
    let taint = |v: Value, slot: crate::ir::StackSlot,
                 points_into: &mut alloc::collections::BTreeMap<Value, Slots>| {
        points_into.entry(v).or_default().insert(slot);
    };

    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            if let InstructionData::StackAddr { stack_slot, .. } = func.dfg.insts[inst] {
                for &r in func.dfg.inst_results(inst) {
                    taint(r, stack_slot, &mut points_into);
                }
            }
        }
    }

    // A slot whose address is used by an instruction that can keep it has
    // escaped.
    let escapes_now = |slots: &Slots, escapes: &mut Slots| {
        for &s in slots {
            escapes.insert(s);
        }
    };

    loop {
        let insts: Vec<Inst> = func
            .layout
            .blocks()
            .flat_map(|b| func.layout.block_insts(b).collect::<Vec<_>>())
            .collect();
        let mut changed = false;
        for inst in insts {
            let data = &func.dfg.insts[inst];
            let op = data.opcode();
            let args: Vec<Value> = data.arguments(&func.dfg.value_lists).to_vec();
            let taint_of = |v: Value,
                            points_into: &alloc::collections::BTreeMap<Value, Slots>| {
                points_into
                    .get(&func.dfg.resolve_aliases(v))
                    .cloned()
                    .unwrap_or_default()
            };
            // The address operand of a load/store is read through, not kept.
            let addr_operand = match data {
                InstructionData::Load { arg, .. } | InstructionData::LoadNoOffset { arg, .. } => {
                    Some(func.dfg.resolve_aliases(*arg))
                }
                InstructionData::Store { args, .. }
                | InstructionData::StoreNoOffset { args, .. } => {
                    Some(func.dfg.resolve_aliases(args[1]))
                }
                _ => None,
            };
            let kept: Slots = args
                .iter()
                .filter(|v| Some(func.dfg.resolve_aliases(**v)) != addr_operand)
                .flat_map(|v| taint_of(*v, &points_into))
                .collect();
            // Successor arguments become block parameters, and the parameters
            // are judged where they are used. This runs for every branching
            // instruction *including* `try_call`/`try_call_indirect`, which are
            // calls and branches at once: the SSA builder appends live
            // variables to their successor arguments, so an address forwarded
            // through one reaches a parameter exactly like a jump's.
            if op.is_branch() {
                for dest in data
                    .branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables)
                    .into_iter()
                {
                    let dest_block = dest.block(&func.dfg.value_lists);
                    let params: Vec<Value> = func.dfg.block_params(dest_block).to_vec();
                    for (param, arg) in params.iter().zip(dest.args(&func.dfg.value_lists)) {
                        let Some(arg) = arg.as_value() else {
                            continue;
                        };
                        for slot in taint_of(arg, &points_into) {
                            let before = points_into.get(param).map_or(0, |s| s.len());
                            taint(*param, slot, &mut points_into);
                            changed |= points_into.get(param).map_or(0, |s| s.len()) != before;
                        }
                    }
                }
            }
            if op.can_load() || op.can_store() || op.is_call() || op.is_return() {
                // Everything except the address operand is kept by someone the
                // analysis cannot see into. For a `try_call` this is its own
                // argument list; its successors were handled above.
                escapes_now(&kept, &mut escapes);
                continue;
            }
            if op.is_branch() {
                continue;
            }
            // Address arithmetic and anything else that computes a derived
            // value keeps the address, so the results inherit it.
            if kept.is_empty() {
                continue;
            }
            if op.can_trap() {
                escapes_now(&kept, &mut escapes);
                continue;
            }
            for &r in func.dfg.inst_results(inst) {
                for slot in kept.iter().copied() {
                    let before = points_into.get(&r).map_or(0, |s| s.len());
                    taint(r, slot, &mut points_into);
                    changed |= points_into.get(&r).map_or(0, |s| s.len()) != before;
                }
            }
        }
        if !changed {
            break;
        }
    }
    AddressInfo {
        escapes,
        points_into,
    }
}

/// Which stack slots an address may point into, per value, plus the set whose
/// address escaped somewhere in the function.
struct AddressInfo {
    escapes: alloc::collections::BTreeSet<crate::ir::StackSlot>,
    /// For a value that *is* a stack address (possibly derived, possibly
    /// arrived through a block parameter), the slots it may point into.
    points_into: alloc::collections::BTreeMap<Value, alloc::collections::BTreeSet<crate::ir::StackSlot>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{Cursor, FuncCursor};
    use crate::ir::types::{I32, I64};
    use crate::ir::{InstBuilder, ValueDef};

    /// The fixtures must be valid CLIF: the pass may assume its input satisfies
    /// the same invariants the compiler's own verifier enforces, so a fixture
    /// the verifier would reject proves nothing.
    fn verify(func: &Function) {
        let flags = crate::settings::Flags::new(crate::settings::builder());
        if let Err(errors) = crate::verifier::verify_function(func, &flags) {
            panic!("fixture is not valid CLIF:\n{}", errors);
        }
    }

    /// True when every value operand is defined in a block that dominates its
    /// use: the SSA invariant Cranelift's register allocator relies on.
    fn ssa_dominates(func: &Function) -> bool {
        let cfg = ControlFlowGraph::with_function(func);
        let domtree = DominatorTree::with_function(func, &cfg);
        for block in func.layout.blocks() {
            for inst in func.layout.block_insts(block) {
                for arg in func.dfg.inst_values(inst) {
                    let arg = func.dfg.resolve_aliases(arg);
                    let def = match func.dfg.value_def(arg) {
                        ValueDef::Param(pb, _) => pb,
                        ValueDef::Result(di, _) => match func.layout.inst_block(di) {
                            Some(b) => b,
                            None => return false,
                        },
                        ValueDef::Union(..) => return false,
                    };
                    if !domtree.block_dominates(def, block) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// A loop header with two predecessors from *outside* the loop. Splitting
    /// only the idom edge leaves the second entry bypassing the new preheader.
    ///
    /// ```text
    /// entry --brif--> header          (entry is the idom and one entry edge)
    ///   \-----------> bx --jump--> header   (a second entry edge)
    /// header --brif--> body --jump--> header   (back edge)
    ///   \-----------> exit
    /// ```
    fn two_entry_loop() -> (Function, Block, Block, Block) {
        let mut func = Function::new();
        func.signature.returns.push(crate::ir::AbiParam::new(I64));
        let entry = func.dfg.make_block();
        let bx = func.dfg.make_block();
        let header = func.dfg.make_block();
        let body = func.dfg.make_block();
        let exit = func.dfg.make_block();

        let mut cur = FuncCursor::new(&mut func);

        cur.insert_block(entry);
        let cond = cur.ins().iconst(I32, 1);
        let k = cur.ins().iconst(I64, 7);
        let one = cur.ins().iconst(I64, 1);
        // Distinct constants per entry: a preheader that mixes the two up
        // forwards the wrong induction value into the loop.
        let from_entry = cur.ins().iconst(I64, 100);
        cur.ins().brif(cond, header, &[from_entry.into()], bx, &[]);

        cur.insert_block(bx);
        let from_bx = cur.ins().iconst(I64, 200);
        cur.ins().jump(header, &[from_bx.into()]);

        cur.insert_block(header);
        let i = cur.func.dfg.append_block_param(header, I64);
        // Invariant: both operands are defined in `entry`, outside the loop.
        let inv = cur.ins().iadd(k, one);
        let cond2 = cur.ins().iconst(I32, 1);
        cur.ins().brif(cond2, body, &[], exit, &[]);

        cur.insert_block(body);
        let _ = cur.ins().iadd(inv, i);
        cur.ins().jump(header, &[i.into()]);

        cur.insert_block(exit);
        cur.ins().return_(&[inv]);

        drop(cur);
        (func, header, entry, bx)
    }

    /// The value a `dest` block is entered with, from whichever terminator.
    fn block_args_into(func: &Function, dest: Block) -> Vec<(Block, Value)> {
        let mut out = Vec::new();
        for block in func.layout.blocks() {
            let Some(term) = func.layout.last_inst(block) else {
                continue;
            };
            let data = &func.dfg.insts[term];
            for dst in data
                .branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables)
                .into_iter()
            {
                if dst.block(&func.dfg.value_lists) != dest {
                    continue;
                }
                for arg in dst.args(&func.dfg.value_lists) {
                    if let Some(v) = arg.as_value() {
                        out.push((block, v));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn licm_routes_every_entry_through_one_preheader() {
        let (mut func, header, entry, bx) = two_entry_loop();
        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");
        let blocks_before = func.layout.blocks().count();

        do_licm(&mut func);

        assert_eq!(
            func.layout.blocks().count(),
            blocks_before + 1,
            "a loop header with two entries still needs a preheader"
        );
        let cfg = ControlFlowGraph::with_function(&func);
        let domtree = DominatorTree::with_function(&func, &cfg);
        // The back edge from `body` is a predecessor too; what matters is that
        // the header keeps exactly one entry from *outside* the loop.
        let outside: Vec<Block> = func
            .layout
            .blocks()
            .filter(|b| cfg.succ_iter(*b).any(|s| s == header))
            .filter(|b| !domtree.block_dominates(header, *b))
            .collect();
        assert_eq!(outside.len(), 1, "the header kept a direct entry edge");
        let preheader = outside[0];
        assert_eq!(
            cfg.succ_iter(preheader).collect::<Vec<_>>(),
            vec![header],
            "the preheader must fall straight into the header"
        );
        assert_eq!(
            cfg.pred_iter(preheader).count(),
            2,
            "both former entries must now go through the preheader"
        );
        // Each entry's own value must arrive at the preheader's parameter: the
        // header's phi argument has to keep coming from the edge that carried
        // it, not from the other one.
        let params = func.dfg.block_params(preheader);
        assert_eq!(params.len(), 1, "the preheader must mirror the header phi");
        let incoming = block_args_into(&func, preheader);
        assert_eq!(incoming.len(), 2, "one argument per entry edge");
        for (pred, value) in incoming {
            let expected = if pred == bx { bx } else { entry };
            assert_eq!(
                def_block(&func, value),
                Some(expected),
                "the preheader parameter took its value from the wrong entry"
            );
        }
        assert!(
            ssa_dominates(&func),
            "the preheader does not dominate every entry to the loop, so a \
             hoisted value is undefined on the other entry path (regalloc \
             reports `EntryLivein`)"
        );
    }

    /// A load from a stack slot whose address never leaves the loop is
    /// invariant even when the loop contains an opaque store: no store can name
    /// that slot. `notrap`-only flags are deliberate -- rakit emits
    /// `load.i64 notrap` for these, without `readonly`/`can_move`.
    #[test]
    fn licm_hoists_a_private_slot_load_past_an_opaque_store() {
        use crate::ir::immediates::Offset32;
        use crate::ir::{MemFlagsData, StackSlotData, StackSlotKind};

        let (mut func, entry, header, body, load, _slot) = private_slot_loop(false);
        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");

        do_licm(&mut func);

        let moved_to = func.layout.inst_block(load).expect("load has a block");
        assert_ne!(moved_to, body, "the private-slot load stayed in the body");
        let cfg = ControlFlowGraph::with_function(&func);
        let domtree = DominatorTree::with_function(&func, &cfg);
        assert!(
            !domtree.block_dominates(header, moved_to),
            "the load moved to {:?}, which is still inside the loop",
            moved_to
        );
        assert_eq!(
            cfg.pred_iter(moved_to).count(),
            1,
            "the hoisted load landed in a block that runs more than once per entry"
        );
        let _ = entry;
        assert!(ssa_dominates(&func), "hoisting broke SSA dominance");
    }

    /// One entry is a conditional branch, the other a jump table. The table
    /// edge cannot be split, so the whole attempt must be abandoned with the
    /// CFG untouched -- redirecting the conditional edge first would leave it
    /// pointing at a block that is not in the layout.
    #[test]
    fn licm_leaves_the_cfg_alone_when_an_entry_is_a_jump_table() {
        use crate::ir::{BlockArg, BlockCall, JumpTableData};

        let mut func = Function::new();
        let entry = func.dfg.make_block();
        let bx = func.dfg.make_block();
        let header = func.dfg.make_block();
        let body = func.dfg.make_block();
        let exit = func.dfg.make_block();
        let mut cur = FuncCursor::new(&mut func);

        cur.insert_block(entry);
        let cond = cur.ins().iconst(I32, 1);
        let five = cur.ins().iconst(I64, 5);
        // Well-formed conditional entry: the header parameter is carried.
        cur.ins()
            .brif(cond, header, &[five.into()], bx, &[]);

        cur.insert_block(bx);
        let idx = cur.ins().iconst(I32, 0);
        let six = cur.ins().iconst(I64, 6);
        let arg: [BlockArg; 1] = [six.into()];
        let mut pool = core::mem::take(&mut cur.func.dfg.value_lists);
        let call = BlockCall::new(header, arg, &mut pool);
        cur.func.dfg.value_lists = pool;
        let table = cur
            .func
            .create_jump_table(JumpTableData::new(call, &[call]));
        cur.ins().br_table(idx, table);

        cur.insert_block(header);
        let i = cur.func.dfg.append_block_param(header, I64);
        let one = cur.ins().iconst(I64, 1);
        let inv = cur.ins().iadd(i, one);
        let cond2 = cur.ins().iconst(I32, 1);
        cur.ins().brif(cond2, body, &[], exit, &[]);

        cur.insert_block(body);
        let _ = cur.ins().iadd(inv, i);
        cur.ins().jump(header, &[i.into()]);

        cur.insert_block(exit);
        cur.ins().return_(&[]);
        drop(cur);

        verify(&func);
        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");
        let before: Vec<Block> = func.layout.blocks().collect();
        do_licm(&mut func);

        assert_eq!(
            func.layout.blocks().collect::<Vec<_>>(),
            before,
            "an unsupported entry must leave the block layout untouched"
        );
        // The conditional entry still targets the header directly.
        let cfg = ControlFlowGraph::with_function(&func);
        assert!(
            cfg.pred_iter(header).any(|p| p.block == entry),
            "the conditional entry was redirected even though the table entry was not"
        );
    }

    /// The loop writes the slot through a *derived* address. `addr_base` cannot
    /// name it, so only the address taint says which slot the store hits; a load
    /// that ignored it would return the value from before the loop.
    #[test]
    fn licm_keeps_a_load_the_loop_writes_through_a_derived_address() {
        use crate::ir::immediates::Offset32;
        use crate::ir::{MemFlagsData, StackSlotData, StackSlotKind};

        let mut func = Function::new();
        let slot = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            16u32,
            0,
        ));
        let entry = func.dfg.make_block();
        let header = func.dfg.make_block();
        let body = func.dfg.make_block();
        let exit = func.dfg.make_block();
        let flags = MemFlagsData::new().with_notrap();
        let mut cur = FuncCursor::new(&mut func);

        cur.insert_block(entry);
        let addr = cur.ins().stack_addr(I64, slot, 0);
        let zero = cur.ins().iconst(I64, 0);
        cur.ins().store(flags, zero, addr, Offset32::new(0));
        cur.ins().jump(header, &[]);

        cur.insert_block(header);
        let cond = cur.ins().iconst(I32, 1);
        cur.ins().brif(cond, body, &[], exit, &[]);

        cur.insert_block(body);
        let load = cur.ins().load(I64, flags, addr, Offset32::new(0));
        let one = cur.ins().iconst(I64, 1);
        let bumped = cur.ins().iadd_imm_u(addr, 0);
        cur.ins().store(flags, one, bumped, Offset32::new(0));
        // Consumed inside the body: the exit block is reachable without the
        // body running, so returning the load from it would not be valid SSA.
        let _ = cur.ins().iadd(load, one);
        cur.ins().jump(header, &[]);

        cur.insert_block(exit);
        cur.ins().return_(&[]);
        drop(cur);
        let load = func.dfg.value_def(load).inst().expect("load result");

        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");
        do_licm(&mut func);

        assert_eq!(
            func.layout.inst_block(load),
            Some(body),
            "a load from a slot the loop writes through a derived address was hoisted"
        );
    }

    /// `try_call` is a call *and* a branch. Its successor arguments carry the
    /// SSA builder's live variables into block parameters, so an address
    /// forwarded that way has to keep its provenance even though the
    /// instruction's own argument list never mentions it. Both successors
    /// matter: the normal return forwards one slot's address, the catch
    /// handler forwards the other's.
    ///
    /// ```text
    /// entry --> header --brif--> ta --try_call--> join(p)  --> header
    ///                     \----> tb --try_call--> cleanup(q) --> header
    /// ```
    #[test]
    fn licm_keeps_a_load_when_an_unwinding_call_forwards_the_address() {
        use crate::ir::immediates::Offset32;
        use crate::ir::{
            AbiParam, BlockArg, BlockCall, ExceptionTableData, ExceptionTableItem, ExceptionTag,
            ExtFuncData, ExternalName, MemFlagsData, Signature, StackSlotData, StackSlotKind,
            UserExternalName,
        };

        let mut func = Function::new();
        let slot_a = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            8u32,
            0,
        ));
        let slot_b = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            8u32,
            0,
        ));
        // Only a calling convention that supports exceptions may be the callee
        // of a `try_call`.
        let sig = func.import_signature(Signature::new(crate::isa::CallConv::SystemV));
        let name = func.declare_imported_user_function(UserExternalName::new(0, 0));
        let callee = func.import_function(ExtFuncData {
            name: ExternalName::user(name),
            signature: sig,
            colocated: false,
            patchable: false,
        });
        let tag = ExceptionTag::from_u32(0);

        let entry = func.dfg.make_block();
        let header = func.dfg.make_block();
        let ta = func.dfg.make_block();
        let tb = func.dfg.make_block();
        let join = func.dfg.make_block();
        let cleanup = func.dfg.make_block();
        let exit = func.dfg.make_block();
        let p = func.dfg.append_block_param(join, I64);
        // A catch handler receives the exception payload first, then the live
        // variables; `q` is the forwarded stack address.
        let _payload0 = func.dfg.append_block_param(cleanup, I64);
        let _payload1 = func.dfg.append_block_param(cleanup, I64);
        let q = func.dfg.append_block_param(cleanup, I64);
        let flags = MemFlagsData::new().with_notrap();
        let mut cur = FuncCursor::new(&mut func);

        cur.insert_block(entry);
        let addr_a = cur.ins().stack_addr(I64, slot_a, 0);
        let addr_b = cur.ins().stack_addr(I64, slot_b, 0);
        cur.ins().jump(header, &[]);

        cur.insert_block(header);
        let cond = cur.ins().iconst(I32, 1);
        cur.ins().brif(cond, ta, &[], tb, &[]);

        // Each path forwards a *different* stack address as a live variable of
        // the call's successors, which is how the SSA builder threads values
        // across an unwinding call.
        for (block, addr) in [(ta, addr_a), (tb, addr_b)] {
            cur.insert_block(block);
            let mut pool = core::mem::take(&mut cur.func.dfg.value_lists);
            let normal = BlockCall::new(join, [BlockArg::Value(addr)], &mut pool);
            let catch = BlockCall::new(
                cleanup,
                [
                    BlockArg::TryCallExn(0),
                    BlockArg::TryCallExn(1),
                    BlockArg::Value(addr_b),
                ],
                &mut pool,
            );
            cur.func.dfg.value_lists = pool;
            let et = cur.func.dfg.exception_tables.push(ExceptionTableData::new(
                sig,
                normal,
                [ExceptionTableItem::Tag(tag, catch)],
            ));
            cur.ins().try_call(callee, &[], et);
        }

        // Both stores are inside the loop, so both loads must stay in it: the
        // first is clobbered through the normal-return parameter, the second
        // through the catch handler's.
        cur.insert_block(join);
        let one = cur.ins().iconst(I64, 1);
        cur.ins().store(flags, one, p, Offset32::new(0));
        let load_a = cur.ins().load(I64, flags, addr_a, Offset32::new(0));
        let _ = cur.ins().iadd(load_a, one);
        cur.ins().jump(header, &[]);

        cur.insert_block(cleanup);
        let one_again = cur.ins().iconst(I64, 1);
        cur.ins().store(flags, one_again, q, Offset32::new(0));
        let load_b = cur.ins().load(I64, flags, addr_b, Offset32::new(0));
        let _ = cur.ins().iadd(load_b, one_again);
        let again = cur.ins().iconst(I32, 1);
        cur.ins().brif(again, header, &[], exit, &[]);

        cur.insert_block(exit);
        cur.ins().return_(&[]);
        drop(cur);
        let load_a = func.dfg.value_def(load_a).inst().expect("load result");
        let load_b = func.dfg.value_def(load_b).inst().expect("load result");

        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");
        do_licm(&mut func);

        assert_eq!(
            func.layout.inst_block(load_a),
            Some(join),
            "a load was hoisted past a store through the normal-return parameter"
        );
        assert_eq!(
            func.layout.inst_block(load_b),
            Some(cleanup),
            "a load was hoisted past a store through the catch handler's parameter"
        );
    }

    /// A call *before* the loop can retain the slot's address and write through
    /// it later, so an in-loop call that never mentions the address still makes
    /// the slot's contents change under the loop. Hoisting the load would then
    /// return the entry value forever.
    #[test]
    fn licm_keeps_a_load_when_the_address_escaped_before_the_loop() {
        use crate::ir::immediates::Offset32;
        use crate::ir::{
            AbiParam, ExtFuncData, ExternalName, MemFlagsData, Signature, StackSlotData,
            StackSlotKind, UserExternalName,
        };

        let mut func = Function::new();
        let slot = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            8u32,
            0,
        ));
        // `retain(*mut u64)` before the loop, `bump()` inside it: the second
        // call has no arguments, so only a function-wide escape analysis can
        // connect it to the slot.
        let mut retain_sig = Signature::new(func.signature.call_conv);
        retain_sig.params.push(AbiParam::new(I64));
        let retain_sig = func.import_signature(retain_sig);
        let call_conv = func.signature.call_conv;
        let bump_sig = func.import_signature(Signature::new(call_conv));
        let retain_name = func.declare_imported_user_function(UserExternalName::new(0, 0));
        let bump_name = func.declare_imported_user_function(UserExternalName::new(0, 1));
        let retain = func.import_function(ExtFuncData {
            name: ExternalName::user(retain_name),
            signature: retain_sig,
            colocated: false,
            patchable: false,
        });
        let bump = func.import_function(ExtFuncData {
            name: ExternalName::user(bump_name),
            signature: bump_sig,
            colocated: false,
            patchable: false,
        });

        let entry = func.dfg.make_block();
        let header = func.dfg.make_block();
        let body = func.dfg.make_block();
        let exit = func.dfg.make_block();
        let flags = MemFlagsData::new().with_notrap();
        let mut cur = FuncCursor::new(&mut func);

        cur.insert_block(entry);
        let addr = cur.ins().stack_addr(I64, slot, 0);
        let zero = cur.ins().iconst(I64, 0);
        let one = cur.ins().iconst(I64, 1);
        cur.ins().store(flags, zero, addr, Offset32::new(0));
        cur.ins().call(retain, &[addr]);
        cur.ins().jump(header, &[]);

        cur.insert_block(header);
        let cond = cur.ins().iconst(I32, 1);
        cur.ins().brif(cond, body, &[], exit, &[]);

        cur.insert_block(body);
        // The loop never stores to the slot, so the only thing that can stop
        // the load being hoisted is the address the entry block handed to
        // `retain`.
        let load = cur.ins().load(I64, flags, addr, Offset32::new(0));
        cur.ins().call(bump, &[]);
        let _ = cur.ins().iadd(load, one);
        cur.ins().jump(header, &[]);

        cur.insert_block(exit);
        cur.ins().return_(&[]);
        drop(cur);
        let load = func.dfg.value_def(load).inst().expect("load result");

        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");
        do_licm(&mut func);

        assert_eq!(
            func.layout.inst_block(load),
            Some(body),
            "a load from a slot whose address a pre-loop call kept was hoisted"
        );
    }

    /// The same loop, but storing to the slot: the load is no longer invariant.
    #[test]
    fn licm_keeps_a_private_slot_load_the_loop_stores() {
        let (mut func, _entry, _header, body, load, slot) = private_slot_loop(true);
        verify(&func);
        assert!(ssa_dominates(&func), "input CFG should already be SSA-dominant");

        do_licm(&mut func);

        assert_eq!(
            func.layout.inst_block(load),
            Some(body),
            "a load of a slot the loop stores to was hoisted (slot {:?})",
            slot
        );
    }

    /// ```text
    /// entry: store 42 -> slot; jump header
    /// header: brif cond -> body, exit
    /// body: load slot; store v -> *p; jump header     (p is a dynamic address)
    /// exit: return
    /// ```
    ///
    /// With `store_slot` the body also stores to `slot`, which makes the load
    /// vary per iteration.
    fn private_slot_loop(
        store_slot: bool,
    ) -> (Function, Block, Block, Block, Inst, crate::ir::StackSlot) {
        use crate::ir::immediates::Offset32;
        use crate::ir::{MemFlagsData, StackSlotData, StackSlotKind};

        let mut func = Function::new();
        let slot = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            8u32,
            0,
        ));
        let entry = func.dfg.make_block();
        let header = func.dfg.make_block();
        let body = func.dfg.make_block();
        let exit = func.dfg.make_block();

        // `notrap` only: `readonly`/`can_move` would make the load hoistable
        // through the generic path and hide the rule under test.
        let flags = MemFlagsData::new().with_notrap();
        let mut cur = FuncCursor::new(&mut func);

        cur.insert_block(entry);
        let addr = cur.ins().stack_addr(I64, slot, 0);
        let forty_two = cur.ins().iconst(I64, 42);
        cur.ins().store(flags, forty_two, addr, Offset32::new(0));
        cur.ins().jump(header, &[]);

        cur.insert_block(header);
        let cond = cur.ins().iconst(I32, 1);
        cur.ins().brif(cond, body, &[], exit, &[]);

        cur.insert_block(body);
        let load = cur.ins().load(I64, flags, addr, Offset32::new(0));
        // A store through an address the pass cannot name: it may write
        // anything it can reach, so only a private slot survives it.
        let dyn_addr = cur.ins().iconst(I64, 0x1000);
        cur.ins().store(flags, load, dyn_addr, Offset32::new(0));
        if store_slot {
            cur.ins().store(flags, load, addr, Offset32::new(0));
        }
        cur.ins().jump(header, &[]);

        cur.insert_block(exit);
        cur.ins().return_(&[]);
        drop(cur);
        let load = func.dfg.value_def(load).inst().expect("load result");
        (func, entry, header, body, load, slot)
    }

    /// `addr_base` must classify the store's **destination** (`args[1]`), not
    /// the stored value (`args[0]`). Each case pins one side: a static value
    /// stored through a dynamic pointer must be `None`, and a dynamic value
    /// stored into a static stack slot must be `Some(StackSlot)`. Reading the
    /// value operand inverts both and fails.
    #[test]
    fn addr_base_uses_store_destination() {
        use crate::ir::immediates::Offset32;
        use crate::ir::{MemFlagsData, StackSlotData, StackSlotKind};

        let mut func = Function::new();
        let entry = func.dfg.make_block();
        let slot = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            8u32,
            0,
        ));
        let flags = MemFlagsData::trusted();
        let mut cur = FuncCursor::new(&mut func);
        cur.insert_block(entry);
        // A statically-known address (the stack slot) and a dynamic pointer (a
        // constant address value, i.e. not a `StackAddr`/`GlobalValue`).
        let static_addr = cur.ins().stack_addr(I64, slot, 0);
        let dyn_addr = cur.ins().iconst(I64, 0x1000);
        let dyn_val = cur.ins().iconst(I64, 42);
        // Static value, dynamic destination.
        let st_static_value_dynamic_dest =
            cur.ins().store(flags, static_addr, dyn_addr, Offset32::new(0));
        // Dynamic value, static destination.
        let st_dynamic_value_static_dest =
            cur.ins().store(flags, dyn_val, static_addr, Offset32::new(0));
        cur.ins().return_(&[]);
        drop(cur);

        assert!(
            addr_base(&func, &func.dfg.insts[st_static_value_dynamic_dest]).is_none(),
            "addr_base classified the stored value, not the dynamic destination"
        );
        assert!(
            matches!(
                addr_base(&func, &func.dfg.insts[st_dynamic_value_static_dest]),
                Some(AddrBase::StackSlot(s)) if s == slot
            ),
            "addr_base missed the static stack-slot destination"
        );
    }
}
