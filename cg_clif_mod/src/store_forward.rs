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

fn disabled() -> bool {
    std::env::var_os("CG_CLIF_NO_STORE_FWD").is_some()
}

pub(crate) fn run(func: &mut ir::Function, endianness: ir::Endianness) {
    if disabled() {
        return;
    }
    if let Ok(skip) = std::env::var("CG_CLIF_SF_SKIP") {
        if func.name.to_string().contains(&skip) {
            return;
        }
    }
    let private = private_slots(func);
    let ptr_bases = private_ptr_bases(func);
    // Facts that hold on every path into a block. Seeding each block with these
    // lets a store in one block reach a load in a dominated successor, which the
    // block-local walk below cannot see.
    let entry = cross_block_entry(func, &private, &ptr_bases);
    for block in func.layout.blocks().collect::<Vec<_>>() {
        forward_block(func, block, &private, &ptr_bases, &entry, endianness);
    }
    drop_dead_stores(func, &private);
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

/// Pointer values whose only use is as the address operand of a load or store,
/// i.e. the address never escapes. A store through such a pointer can be
/// forwarded to a later load of the same base even though the pass cannot prove
/// anything about the memory it points at: the address is identical, so no alias
/// can interpose. This is how `core::hash::sip`'s `State` (`&mut self`) is
/// handled — cg_clif keeps the state in the caller's frame and reaches it through
/// the opaque parameter rather than a private stack slot.
fn private_ptr_bases(func: &ir::Function) -> FxHashSet<Value> {
    let dfg = &func.dfg;
    let mut addr_only: FxHashSet<Value> = FxHashSet::default();
    let mut escaping: FxHashSet<Value> = FxHashSet::default();

    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let opcode = dfg.insts[inst].opcode();
            let addr_index = address_index(opcode);
            for (i, &arg) in dfg.insts[inst].arguments(&dfg.value_lists).iter().enumerate() {
                let arg = dfg.resolve_aliases(arg);
                if addr_index == Some(i) {
                    addr_only.insert(arg);
                } else {
                    escaping.insert(arg);
                }
            }
            for dest in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                for arg in dest.args(&dfg.value_lists) {
                    if let Some(v) = arg.as_value() {
                        escaping.insert(dfg.resolve_aliases(v));
                    }
                }
            }
        }
    }
    addr_only.retain(|v| !escaping.contains(v));
    addr_only
}

/// Instructions that may write memory the pass does not track: calls, atomics,
/// fences and the like. A plain store is handled separately by [`record_store`].
fn clobbers_memory(opcode: Opcode) -> bool {
    opcode.is_call() || opcode.other_side_effects()
}

/// Record the fact that `inst` stores its data operand to `avail`, invalidating
/// the facts it may overwrite.
fn record_store(
    func: &ir::Function,
    inst: Inst,
    args: &[Value],
    avail: &mut Avail,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
) {
    let Some(addr) = args.get(1).copied() else { return };
    let value = args[0];
    let ty = func.dfg.value_type(value);
    let Some((base, off)) = mem_key(func, inst, addr).filter(|k| tracked(k, private, ptr_bases))
    else {
        // Store through an address the pass does not track: it may alias any
        // tracked pointer base, so every pointer fact is invalidated. A private
        // stack slot cannot be reached from an untracked address, so it stays.
        avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
        return;
    };
    // An atomic store is a synchronisation point and cannot be forwarded across;
    // the clobber above already dropped the pointer facts, so do not re-add one.
    if matches!(base, Base::Ptr(_)) && func.dfg.insts[inst].opcode() != Opcode::Store {
        return;
    }
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

/// Slots whose address is only ever used as the address operand of a load/store.
/// Anything else (passed to a call, returned, stored as data, arithmetic) means
/// the address may escape, and the slot is left alone.
fn private_slots(func: &ir::Function) -> FxHashSet<StackSlot> {
    let dfg = &func.dfg;
    let mut escaping: FxHashSet<StackSlot> = FxHashSet::default();
    let mut slots: FxHashSet<StackSlot> = FxHashSet::default();

    // A `stack_addr` value that resolves to a slot, if `arg` is one.
    let slot_of = |arg: Value| -> Option<StackSlot> {
        let arg = dfg.resolve_aliases(arg);
        let ir::ValueDef::Result(def_inst, 0) = dfg.value_def(arg) else { return None };
        let ir::InstructionData::StackAddr { stack_slot, .. } = dfg.insts[def_inst] else {
            return None;
        };
        Some(stack_slot)
    };

    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let opcode = dfg.insts[inst].opcode();
            let addr_index = address_index(opcode);
            for (i, &arg) in dfg.insts[inst].arguments(&dfg.value_lists).iter().enumerate() {
                let Some(slot) = slot_of(arg) else { continue };
                slots.insert(slot);
                if addr_index != Some(i) {
                    escaping.insert(slot);
                }
            }
            // Branch arguments are *not* part of `arguments()`. An address passed
            // through a block parameter (a phi) is indistinguishable from an
            // escaping address by this pass, so treat it as one: otherwise a load
            // that reads the slot through the parameter is invisible and the
            // store it depends on is wrongly deleted.
            for dest in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
                for arg in dest.args(&dfg.value_lists) {
                    if let Some(slot) = arg.as_value().and_then(slot_of) {
                        slots.insert(slot);
                        escaping.insert(slot);
                    }
                }
            }
        }
    }
    slots.retain(|s| !escaping.contains(s));
    slots
}

/// Forward stores to loads within a single block. A store is only remembered for
/// a private slot, so an intervening call or indirect store cannot invalidate it.
/// `entry` holds the facts that hold on every path into `block`.
fn forward_block(
    func: &mut ir::Function,
    block: Block,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
    entry: &FxHashMap<Block, Avail>,
    endianness: ir::Endianness,
) {
    let mut avail: Avail = entry.get(&block).cloned().unwrap_or_default();

    for inst in func.layout.block_insts(block).collect::<Vec<_>>() {
        let opcode = func.dfg.insts[inst].opcode();
        let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists).to_vec();

        if clobbers_memory(opcode) {
            avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
            continue;
        }
        if opcode.can_store() {
            record_store(func, inst, &args, &mut avail, private, ptr_bases);
        } else if opcode.can_load() {
            let Some(addr) = args.first().copied() else { continue };
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
    block: Block,
    mut avail: Avail,
    private: &FxHashSet<StackSlot>,
    ptr_bases: &FxHashSet<Value>,
) -> Avail {
    for inst in func.layout.block_insts(block) {
        let opcode = func.dfg.insts[inst].opcode();
        if clobbers_memory(opcode) {
            avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
            continue;
        }
        if !opcode.can_store() {
            continue;
        }
        let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists).to_vec();
        record_store(func, inst, &args, &mut avail, private, ptr_bases);
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
) -> FxHashMap<Block, Avail> {
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let blocks: Vec<Block> = func.layout.blocks().collect();
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

    let mut out: FxHashMap<Block, Avail> = FxHashMap::default();
    let mut entry: FxHashMap<Block, Avail> = FxHashMap::default();
    loop {
        let mut changed = false;
        for &block in &blocks {
            let in_avail = meet(&out, block);
            let new_out = transfer(func, block, in_avail.clone(), private, ptr_bases);
            if out.get(&block) != Some(&new_out) {
                out.insert(block, new_out);
                changed = true;
            }
            entry.insert(block, in_avail);
        }
        if !changed {
            break;
        }
    }
    entry
}

/// Remove stores into private slots whose bytes are not read by any surviving
/// load. Because the slot cannot be observed from outside, such a store has no
/// effect.
fn drop_dead_stores(func: &mut ir::Function, private: &FxHashSet<StackSlot>) {
    let mut read: Vec<(StackSlot, i64, i64)> = Vec::new();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            if !func.dfg.insts[inst].opcode().can_load() {
                continue;
            }
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists).to_vec();
            let Some(key) = args.first().copied().and_then(|a| stack_key(func, inst, a)) else {
                continue;
            };
            let size = func
                .dfg
                .inst_results(inst)
                .first()
                .map(|&r| i64::from(func.dfg.value_type(r).bytes()))
                .unwrap_or(0);
            read.push((key.0, key.1, size));
        }
    }

    let mut dead: Vec<Inst> = Vec::new();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            if !func.dfg.insts[inst].opcode().can_store() {
                continue;
            }
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists).to_vec();
            let Some(key) = args.get(1).copied().and_then(|a| stack_key(func, inst, a)) else {
                continue;
            };
            if !private.contains(&key.0) {
                continue;
            }
            let size = i64::from(func.dfg.value_type(args[0]).bytes());
            let overlaps = read
                .iter()
                .any(|&(slot, off, len)| slot == key.0 && off < key.1 + size && key.1 < off + len);
            if !overlaps {
                dead.push(inst);
            }
        }
    }
    for inst in dead {
        func.layout.remove_inst(inst);
    }
}
