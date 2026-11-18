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

use cranelift_codegen::ir::{self, Block, Inst, Opcode, StackSlot, Value, types};
use rustc_data_structures::fx::{FxHashMap, FxHashSet};

/// Byte range written or read at a stack slot.
type Key = (StackSlot, i64);

fn disabled() -> bool {
    std::env::var_os("CG_CLIF_NO_STORE_FWD").is_some()
}

pub(crate) fn run(func: &mut ir::Function) {
    if disabled() {
        return;
    }
    if let Ok(skip) = std::env::var("CG_CLIF_SF_SKIP") {
        if func.name.to_string().contains(&skip) {
            return;
        }
    }
    let private = private_slots(func);
    if private.is_empty() {
        return;
    }
    for block in func.layout.blocks().collect::<Vec<_>>() {
        forward_block(func, block, &private);
    }
    drop_dead_stores(func, &private);
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
fn stack_key(func: &ir::Function, inst: Inst, addr: Value) -> Option<Key> {
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
fn forward_block(func: &mut ir::Function, block: Block, private: &FxHashSet<StackSlot>) {
    let mut avail: FxHashMap<Key, (Value, types::Type)> = FxHashMap::default();

    for inst in func.layout.block_insts(block).collect::<Vec<_>>() {
        let opcode = func.dfg.insts[inst].opcode();
        let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists).to_vec();

        if opcode.can_store() {
            let Some(key) = args.get(1).copied().and_then(|a| stack_key(func, inst, a)) else {
                continue;
            };
            if private.contains(&key.0) {
                let value = args[0];
                let ty = func.dfg.value_type(value);
                let (start, end) = (key.1, key.1 + i64::from(ty.bytes()));
                avail.retain(|&(slot, off), &mut (_, prev_ty)| {
                    let prev_end = off + i64::from(prev_ty.bytes());
                    slot != key.0 || end <= off || prev_end <= start
                });
                avail.insert(key, (value, ty));
            }
        } else if opcode.can_load() {
            let Some(key) = args.first().copied().and_then(|a| stack_key(func, inst, a)) else {
                continue;
            };
            let Some(&(value, ty)) = avail.get(&key) else { continue };
            let results = func.dfg.inst_results(inst);
            if results.len() != 1 || func.dfg.value_type(results[0]) != ty {
                continue;
            }
            let result = results[0];
            func.dfg.detach_inst_results(inst);
            func.dfg.change_to_alias(result, value);
            func.layout.remove_inst(inst);
        }
    }
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
