//! Stack store-to-load forwarding on CLIF.
//!
//! cg_clif materialises aggregates (`Option<u64>`, tuples, `Vec` headers, …) by
//! writing them into an explicit stack slot and reading them back with a load.
//! Cranelift's egraph only value-numbers loads that are `readonly`, `notrap` and
//! `can_move`, and a store to an explicit slot is not `readonly`, so a
//! `store`/`load` round-trip of the same address survives all the way into the
//! machine code. Inside a loop that puts the load on the carried dependency
//! chain, which is exactly the deficit measured against LLVM on the
//! `Option`-in-loop benchmark.
//!
//! This pass forwards such a store to the load that reads the same address and
//! then drops stores whose bytes nothing reads any more. Both rewrites are
//! restricted to stack slots whose address never escapes the function, so no
//! call or indirect store can observe the slot between the store and the load.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::ir::{self, Block, Inst, MemFlagsData, Opcode, StackSlot, Value, types, InstBuilder};
use rustc_data_structures::fx::{FxHashMap, FxHashSet};

/// A memory location a store writes or a load reads: either a private stack
/// slot or a non-escaping pointer value, each with a byte offset.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Base {
    Slot(StackSlot),
    Ptr(Value),
}

/// Byte range written or read at a base.
type Key = (Base, i64);

/// For each byte range, the SSA value last stored to it.
type Avail = FxHashMap<Key, (Value, types::Type)>;

pub(crate) fn run(func: &mut ir::Function, endianness: ir::Endianness) {
    if !crate::tunables::store_forward_enabled() {
        return;
    }
    let dbg = crate::tunables::store_forward_time();
    let t0 = std::time::Instant::now();
    if let Some(skip) = crate::tunables::store_forward_skip() {
        if func.name.to_string().contains(skip) {
            return;
        }
    }
    let Bases { private_slots: private, ptr_bases, loaded } = analyze(func);
    let t1 = std::time::Instant::now();
    // Nothing the pass can track: no private slot and no non-escaping pointer
    // base means no store is forwardable and no dead store can be identified.
    if private.is_empty() && ptr_bases.is_empty() {
        return;
    }
    // Facts that hold on every path into a block. Seeding each block with these
    // lets a store in one block reach a load in a dominated successor, which the
    // block-local walk below cannot see. A single-block function has no successor
    // to seed, so the dataflow is skipped.
    let entry = if !loaded.is_empty()
        && func.layout.blocks().nth(1).is_some()
        && crate::tunables::store_forward_cross_block()
        && cbe_within_budget(func)
    {
        cross_block_entry(func, &private, &ptr_bases, &loaded)
    } else {
        FxHashMap::default()
    };
    let t2 = std::time::Instant::now();
    if !loaded.is_empty() {
        for block in func.layout.blocks().collect::<Vec<_>>() {
            forward_block(func, block, &private, &ptr_bases, &loaded, &entry, endianness);
        }
    }
    let t3 = std::time::Instant::now();
    drop_dead_stores(func, &private);
    if dbg {
        eprintln!(
            "[sf] {} insts={} setup={:?} cbe={:?} fwd={:?} dead={:?}",
            func.name,
            func.dfg.num_insts(),
            t1 - t0,
            t2 - t1,
            t3 - t2,
            t3.elapsed(),
        );
    }
}

/// The stack slot a `stack_addr` value (already alias-resolved) refers to.
fn slot_of_resolved(dfg: &ir::DataFlowGraph, arg: Value) -> Option<StackSlot> {
    let ir::ValueDef::Result(def_inst, 0) = dfg.value_def(arg) else { return None };
    let ir::InstructionData::StackAddr { stack_slot, .. } = dfg.insts[def_inst] else {
        return None;
    };
    Some(stack_slot)
}

/// If `addr` is a `stack_addr` of a known slot, or a tracked pointer base,
/// return the base and the byte offset at which `inst` accesses it.
fn mem_key(func: &ir::Function, inst: Inst, addr: Value) -> Option<Key> {
    let addr = func.dfg.resolve_aliases(addr);
    if let ir::ValueDef::Result(def_inst, 0) = func.dfg.value_def(addr) {
        if let ir::InstructionData::StackAddr { stack_slot, offset, .. } = func.dfg.insts[def_inst] {
            let off = i64::from(offset) + offset_of(func, inst)?;
            return Some((Base::Slot(stack_slot), off));
        }
    }
    Some((Base::Ptr(addr), offset_of(func, inst)?))
}

/// Is this key a location the pass is allowed to track?
fn tracked(key: &Key, private: &FxHashSet<StackSlot>, ptr_bases: &FxHashSet<Value>) -> bool {
    match key.0 {
        Base::Slot(s) => private.contains(&s),
        Base::Ptr(p) => ptr_bases.contains(&p),
    }
}


/// The two escape analyses plus the set of byte ranges any load reads.
///
/// All three need a walk over every instruction, so they are computed together:
/// three separate walks cost more than the rest of the pass on medium functions.
struct Bases {
    /// Stack slots whose address never escapes.
    private_slots: FxHashSet<StackSlot>,
    /// Pointer values that are only ever used as a load/store address.
    ptr_bases: FxHashSet<Value>,
    /// `(base, offset)` byte ranges a load reads.
    loaded: FxHashSet<Key>,
}

fn analyze(func: &ir::Function) -> Bases {
    let dfg = &func.dfg;
    let mut escaping_slots: FxHashSet<StackSlot> = FxHashSet::default();
    let mut slots: FxHashSet<StackSlot> = FxHashSet::default();
    let mut addr_only: FxHashSet<Value> = FxHashSet::default();
    let mut escaping_ptr: FxHashSet<Value> = FxHashSet::default();
    // Candidate loads; filtered against the final private sets below.
    let mut loads: Vec<(Inst, Value)> = Vec::new();

    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let opcode = dfg.insts[inst].opcode();
            let addr_index = address_index(opcode);
            for (i, &arg) in dfg.insts[inst].arguments(&dfg.value_lists).iter().enumerate() {
                // Resolve once: `slot_of` and the pointer classification both
                // need the alias-resolved value, and resolving twice dominated
                // the walk.
                let arg = dfg.resolve_aliases(arg);
                if let Some(slot) = slot_of_resolved(dfg, arg) {
                    slots.insert(slot);
                    if addr_index != Some(i) {
                        escaping_slots.insert(slot);
                    }
                }
                if addr_index == Some(i) {
                    addr_only.insert(arg);
                } else {
                    escaping_ptr.insert(arg);
                }
            }
            // Branch arguments are *not* part of `arguments()`. An address passed
            // through a block parameter (a phi) is indistinguishable from an
            // escaping address by this pass, so treat it as one: otherwise a load
            // that reads the slot through the parameter is invisible and the
            // store it depends on is wrongly deleted.
            for dest in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                for arg in dest.args(&dfg.value_lists) {
                    if let Some(v) = arg.as_value() {
                        let v = dfg.resolve_aliases(v);
                        if let Some(slot) = slot_of_resolved(dfg, v) {
                            slots.insert(slot);
                            escaping_slots.insert(slot);
                        }
                        escaping_ptr.insert(v);
                    }
                }
            }
            if opcode.can_load() {
                if let Some(addr) = dfg.insts[inst].arguments(&dfg.value_lists).first().copied() {
                    loads.push((inst, addr));
                }
            }
        }
    }

    slots.retain(|s| !escaping_slots.contains(s));
    addr_only.retain(|v| !escaping_ptr.contains(v));

    let mut loaded: FxHashSet<Key> = FxHashSet::default();
    for (inst, addr) in loads {
        if let Some(key) = mem_key(func, inst, addr) {
            if tracked(&key, &slots, &addr_only) {
                loaded.insert(key);
            }
        }
    }

    Bases { private_slots: slots, ptr_bases: addr_only, loaded }
}

/// Instructions that may write memory the pass does not track: calls, atomics,
/// fences and the like. A plain store is handled separately by [`record_store`].
fn clobbers_memory(opcode: Opcode) -> bool {
    opcode.is_call() || opcode.other_side_effects()
}

/// A memory effect that matters to the store dataflow.
enum SfEvent {
    /// A call, atomic, fence, …: drops every pointer fact.
    Clobber,
    /// A store of `value` to `addr`; `inst` is kept for its type and opcode.
    Store { inst: Inst, value: Value, addr: Value },
}

/// The store events of a block, decoded once. The dataflow revisits blocks many
/// times, so resolving aliases and re-reading instruction data on every visit is
/// the dominant cost of the pass.
fn block_events(func: &ir::Function, block: Block) -> Vec<SfEvent> {
    let mut events = Vec::new();
    for inst in func.layout.block_insts(block) {
        let opcode = func.dfg.insts[inst].opcode();
        if clobbers_memory(opcode) {
            events.push(SfEvent::Clobber);
        } else if opcode.can_store() {
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists);
            if args.len() >= 2 {
                events.push(SfEvent::Store { inst, value: args[0], addr: args[1] });
            }
        }
    }
    events
}

/// Whether the function is small enough that the cross-block dataflow is worth
/// running. The dataflow costs time proportional to blocks × facts, while the
/// forwarding it enables mostly matters in the small hot helpers; a large
/// function keeps the block-local result, which is what it would have had before.
fn cbe_within_budget(func: &ir::Function) -> bool {
    func.dfg.num_insts() <= crate::tunables::store_forward_cbe_max()
}

/// Largest function (in CLIF instructions) the cross-block dataflow runs on.
pub(crate) const CBE_MAX_INSTS: usize = 300;


/// Record the fact that `inst` stores its data operand to `avail`, invalidating
/// the facts it may overwrite.
fn record_store(
    func: &ir::Function,
    inst: Inst,
    value: Value,
    addr: Value,
    avail: &mut Avail,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
    loaded: &FxHashSet<Key>,
) {
    let ty = func.dfg.value_type(value);
    let Some((base, off)) = mem_key(func, inst, addr).filter(|k| tracked(k, private, ptr_bases))
    else {
        // Store through an address the pass does not track: it may alias any
        // tracked pointer base, so every pointer fact is invalidated. A private
        // stack slot cannot be reached from an untracked address, so it stays.
        avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
        return;
    };
    // Invalidate every fact this store overwrites before deciding whether the
    // store itself is worth recording: a store the pass does not track (not in
    // `loaded`, or an atomic store) still writes the bytes, so a stale fact must
    // not survive it.
    let (start, end) = (off, off + i64::from(ty.bytes()));
    avail.retain(|&(b, o), &mut (_, prev_ty)| {
        if b == base {
            let prev_end = o + i64::from(prev_ty.bytes());
            end <= o || prev_end <= start
        } else {
            // A tracked-pointer store may alias a *different* pointer base, but
            // never a private stack slot; a private-slot store aliases nothing.
            !(matches!(base, Base::Ptr(_)) && matches!(b, Base::Ptr(_)))
        }
    });
    // An atomic store is a synchronisation point and cannot be forwarded across;
    // the invalidation above already dropped the pointer facts, so do not re-add
    // one.
    if matches!(base, Base::Ptr(_)) && func.dfg.insts[inst].opcode() != Opcode::Store {
        return;
    }
    // A store no load can read is never forwarded, so it need not be recorded.
    if !loaded.contains(&(base, off)) {
        return;
    }
    avail.insert((base, off), (value, ty));
}

/// The address operand index of a load/store, i.e. the one argument that is a
/// pointer *read* rather than a pointer *captured*.
fn address_index(opcode: Opcode) -> Option<usize> {
    match opcode {
        Opcode::Load => Some(0),
        Opcode::Store => Some(1),
        _ => None,
    }
}

fn offset_of(func: &ir::Function, inst: Inst) -> Option<i64> {
    match func.dfg.insts[inst] {
        ir::InstructionData::Load { offset, .. } | ir::InstructionData::Store { offset, .. } => {
            Some(i64::from(offset))
        }
        _ => Some(0),
    }
}

/// If `addr` is a `stack_addr` of a known slot, return `(slot, offset)` where
/// `offset` is the byte offset from the slot base at which `inst` accesses it.
fn stack_key(func: &ir::Function, inst: Inst, addr: Value) -> Option<(StackSlot, i64)> {
    let addr = func.dfg.resolve_aliases(addr);
    let ir::ValueDef::Result(def_inst, 0) = func.dfg.value_def(addr) else { return None };
    let ir::InstructionData::StackAddr { stack_slot, offset, .. } = func.dfg.insts[def_inst]
    else {
        return None;
    };
    let off = i64::from(offset) + offset_of(func, inst)?;
    Some((stack_slot, off))
}


/// Forward stores to loads within a single block. A store is only remembered for
/// a private slot, so an intervening call or indirect store cannot invalidate it.
/// `entry` holds the facts that hold on every path into `block`.
fn forward_block(
    func: &mut ir::Function,
    block: Block,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
    loaded: &FxHashSet<Key>,
    entry: &FxHashMap<Block, Avail>,
    endianness: ir::Endianness,
) {
    let mut avail: Avail = entry.get(&block).cloned().unwrap_or_default();

    for inst in func.layout.block_insts(block).collect::<Vec<_>>() {
        let opcode = func.dfg.insts[inst].opcode();

        if clobbers_memory(opcode) {
            avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
            continue;
        }
        if opcode.can_store() {
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists);
            if args.len() >= 2 {
                let (value, addr) = (args[0], args[1]);
                record_store(func, inst, value, addr, &mut avail, private, ptr_bases, loaded);
            }
        } else if opcode.can_load() {
            let addr = func.dfg.insts[inst].arguments(&func.dfg.value_lists).first().copied();
            let Some(addr) = addr else { continue };
            let Some(key) = mem_key(func, inst, addr) else { continue };
            if !tracked(&key, private, ptr_bases) {
                continue;
            }
            let Some(&(value, ty)) = avail.get(&key) else { continue };
            let results = func.dfg.inst_results(inst);
            if results.len() != 1 {
                continue;
            }
            let result = results[0];
            let result_ty = func.dfg.value_type(result);
            if result_ty == ty {
                func.dfg.detach_inst_results(inst);
                func.dfg.change_to_alias(result, value);
                func.layout.remove_inst(inst);
            } else if result_ty.bytes() == ty.bytes()
                && result_ty.is_vector()
                && ty.is_vector()
            {
                // A store and a load of the same bytes but different vector types is
                // exactly how a SIMD bitcast (`__m128i` <-> `Simd<i8, 16>`) compiles:
                // the value is stored as one lane type and read back as another.
                // Forward the stored value through a bitcast instead of leaving the
                // round-trip for the machine code.
                let mut pos = FuncCursor::new(func);
                pos.goto_inst(inst);
                let flags = MemFlagsData::new().with_endianness(endianness);
                let cast = pos.ins().bitcast(result_ty, flags, value);
                pos.func.dfg.detach_inst_results(inst);
                pos.func.dfg.change_to_alias(result, cast);
                pos.func.layout.remove_inst(inst);
            }
        }
    }
}

/// Run the block-local store transfer over `avail` without rewriting anything.
/// Used by the cross-block dataflow, which only needs the store facts.
fn transfer(
    func: &ir::Function,
    events: &[SfEvent],
    mut avail: Avail,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
    loaded: &FxHashSet<Key>,
) -> Avail {
    for event in events {
        match *event {
            SfEvent::Clobber => {
                avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
            }
            SfEvent::Store { inst, value, addr } => {
                record_store(func, inst, value, addr, &mut avail, private, ptr_bases, loaded);
            }
        }
    }
    avail
}

/// The store facts that hold on *every* path into each block, i.e. the meet over
/// all predecessors. A fact survives the meet only when every predecessor agrees
/// on the exact same SSA value, so the value stored is well defined at the merge
/// point and dominates it.
fn cross_block_entry(
    func: &ir::Function,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
    loaded: &FxHashSet<Key>,
) -> FxHashMap<Block, Avail> {
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let blocks: Vec<Block> = func.layout.blocks().collect();
    // Decode each block's store events once; the dataflow below revisits blocks.
    let events: FxHashMap<Block, Vec<SfEvent>> =
        blocks.iter().map(|&b| (b, block_events(func, b))).collect();
    // The entry block is also reached from outside the function, where no store
    // has run. The CFG does not model that edge, so a self-loop on the entry
    // block would otherwise leak facts from the back edge into its own entry.
    let entry_block = func.layout.entry_block();

    let meet = |out: &FxHashMap<Block, Avail>, block: Block| -> Avail {
        if Some(block) == entry_block {
            return Avail::default();
        }
        let mut acc: Option<Avail> = None;
        for pred in cfg.pred_iter(block) {
            let po = out.get(&pred.block).cloned().unwrap_or_default();
            match &mut acc {
                None => acc = Some(po),
                Some(cur) => cur.retain(|k, v| po.get(k) == Some(v)),
            }
        }
        acc.unwrap_or_default()
    };

    // One reverse-postorder sweep. For a reducible CFG — everything cg_clif
    // emits — a block's predecessors other than back edges are visited before
    // it, so a single pass reaches the same facts as a fixed point. A back edge
    // is not yet available, so it contributes no facts: the result is a
    // conservative under-approximation, which is always safe for forwarding.
    let mut entry: FxHashMap<Block, Avail> = FxHashMap::default();
    let mut out: FxHashMap<Block, Avail> = FxHashMap::default();
    for block in reverse_postorder(&cfg, entry_block, &blocks) {
        if Some(block) == entry_block {
            continue;
        }
        let in_avail = meet(&out, block);
        entry.insert(block, in_avail.clone());
        let new_out = transfer(func, &events[&block], in_avail, private, ptr_bases, loaded);
        out.insert(block, new_out);
    }
    entry
}

/// Blocks in reverse postorder, reachable from the entry block. Blocks that are
/// not reachable keep their default (empty) entry facts.
fn reverse_postorder(
    cfg: &cranelift_codegen::flowgraph::ControlFlowGraph,
    entry: Option<Block>,
    blocks: &[Block],
) -> Vec<Block> {
    let Some(entry) = entry else { return blocks.to_vec() };
    let mut visited: FxHashSet<Block> = FxHashSet::default();
    let mut postorder: Vec<Block> = Vec::with_capacity(blocks.len());
    // Iterative DFS; the `(block, next-succ)` stack mirrors the recursion.
    let mut stack: Vec<(Block, usize)> = vec![(entry, 0)];
    let succs: FxHashMap<Block, Vec<Block>> =
        blocks.iter().map(|&b| (b, cfg.succ_iter(b).collect())).collect();
    visited.insert(entry);
    while let Some(&mut (block, ref mut i)) = stack.last_mut() {
        let next = succs[&block].get(*i).copied();
        match next {
            Some(succ) => {
                *i += 1;
                if visited.insert(succ) {
                    stack.push((succ, 0));
                }
            }
            None => {
                postorder.push(block);
                stack.pop();
            }
        }
    }
    postorder.reverse();
    postorder
}

/// Remove stores into private slots whose bytes are not read by any surviving
/// load. Because the slot cannot be observed from outside, such a store has no
/// effect.
fn drop_dead_stores(func: &mut ir::Function, private: &FxHashSet<StackSlot>) {
    // Reads bucketed by slot: a store only has to be compared against the reads
    // of its own slot, not against every load in the function.
    let mut reads: FxHashMap<StackSlot, Vec<(i64, i64)>> = FxHashMap::default();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            if !func.dfg.insts[inst].opcode().can_load() {
                continue;
            }
            let addr = func.dfg.insts[inst].arguments(&func.dfg.value_lists).first().copied();
            let Some(key) = addr.and_then(|a| stack_key(func, inst, a)) else {
                continue;
            };
            let size = func
                .dfg
                .inst_results(inst)
                .first()
                .map(|&r| i64::from(func.dfg.value_type(r).bytes()))
                .unwrap_or(0);
            reads.entry(key.0).or_default().push((key.1, size));
        }
    }

    let mut dead: Vec<Inst> = Vec::new();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            if !func.dfg.insts[inst].opcode().can_store() {
                continue;
            }
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists);
            let Some(key) = args.get(1).copied().and_then(|a| stack_key(func, inst, a)) else {
                continue;
            };
            if !private.contains(&key.0) {
                continue;
            }
            let size = i64::from(func.dfg.value_type(args[0]).bytes());
            let overlaps = reads
                .get(&key.0)
                .is_some_and(|rs| rs.iter().any(|&(off, len)| off < key.1 + size && key.1 < off + len));
            if !overlaps {
                dead.push(inst);
            }
        }
    }
    for inst in dead {
        func.layout.remove_inst(inst);
    }
}
