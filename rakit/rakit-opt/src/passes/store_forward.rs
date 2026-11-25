//! Stack store-to-load forwarding on RAKIT.
//!
//! rakit materialises aggregates (`Option<u64>`, tuples, `Vec` headers, …) by
//! writing them into an explicit stack slot and reading them back with a load.
//! Rakit's egraph only value-numbers loads that are `readonly`, `notrap` and
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

use std::collections::{HashMap as FxHashMap, HashSet as FxHashSet};

use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::{
    self, Block, Inst, InstBuilder, MemFlagsData, Opcode, StackSlot, Value, types,
};

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

/// A pointer value that provably points inside one stack slot, with an upper
/// bound on its displacement from the slot base.
///
/// Two such pointers derived from *different* slots cannot alias, which lets a
/// store through one keep the facts of the other. Without the bound the
/// displacement could leave the slot and land in a neighbour, so a fact is only
/// trusted when its access stays inside the slot.
#[derive(Clone, Copy)]
struct Confined {
    slot: StackSlot,
    bound: u64,
    size: u64,
}

impl Confined {
    fn covers(&self, off: i64, ty: types::Type) -> bool {
        off >= 0 && self.bound + off as u64 + u64::from(ty.bytes()) <= self.size
    }
}

/// Memoises the recursive address queries for the function being processed.
/// Both answers are safe to reuse at a smaller depth: a cached `Some` is a proof
/// that does not depend on the budget, and a cached `None` only ever makes the
/// pass more conservative.
#[derive(Default)]
struct AddrCache {
    bound: FxHashMap<Value, Option<u64>>,
}

#[derive(Default)]
struct ProvScratch {
    seen: FxHashSet<Value>,
    work: Vec<Value>,
}

thread_local! {
    static ADDR_CACHE: std::cell::RefCell<AddrCache> = std::cell::RefCell::default();
    static PROV_SCRATCH: std::cell::RefCell<ProvScratch> = std::cell::RefCell::default();
}

/// Values one provenance query may visit before the pass gives up on the
/// function. The walk follows SSA definitions, so it terminates by itself; the
/// cap only bounds a pathological input.
const PROV_MAX_NODES: usize = 4096;

/// Visit every stack slot `v` may derive from: the union over all operands of
/// the address arithmetic that built it.
///
/// Answering with only the *first* slot found is not enough for an escape
/// decision: `low + addr(b)`, where `low` is a casted offset computed from `a`'s
/// address, derives from both `a` and `b`, and a call receiving that sum can read
/// either slot. A depth budget is not enough either: the walk has to reach the
/// `stack_addr` however many operations sit between it and the use, and a shared
/// subexpression must not be skipped because a longer path reached it first.
/// The worklist therefore runs to completion over SSA definitions, visiting each
/// value once.
///
/// Returns false when [`PROV_MAX_NODES`] was exceeded, i.e. the set is
/// incomplete: the caller must then treat *no* slot as private rather than
/// assume the unvisited ones are absent.
fn for_each_provenance_slot(
    func: &ir::Function,
    v: Value,
    mut f: impl FnMut(StackSlot),
) -> bool {
    PROV_SCRATCH.with(|cell| {
        let mut scratch = cell.borrow_mut();
        let ProvScratch { seen, work } = &mut *scratch;
        seen.clear();
        work.clear();
        work.push(v);
        let mut visited = 0usize;
        while let Some(v) = work.pop() {
            let v = func.dfg.resolve_aliases(v);
            if !seen.insert(v) {
                continue;
            }
            visited += 1;
            if visited > PROV_MAX_NODES {
                return false;
            }
            let ir::ValueDef::Result(inst, 0) = func.dfg.value_def(v) else { continue };
            let data = &func.dfg.insts[inst];
            match data {
                ir::InstructionData::StackAddr { stack_slot, .. } => f(*stack_slot),
                _ if pure_address_arith(data.opcode()) => {
                    work.extend(data.arguments(&func.dfg.value_lists).iter().copied());
                }
                _ => {}
            }
        }
        true
    })
}

/// Integer arithmetic that cannot leak an address out of the function: the
/// result is another address (or a value derived from one) that the pass can
/// still attribute to the same slot.
fn pure_address_arith(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Iadd
            | Opcode::Isub
            | Opcode::Imul
            | Opcode::Band
            | Opcode::Bor
            | Opcode::Bxor
            | Opcode::Uextend
            | Opcode::Sextend
            | Opcode::Ireduce
            | Opcode::Bitcast
    )
}

/// Upper bound on the value of `v` when it is built from constants, masks and
/// add/subtract. `band(x, M)` is bounded by `M` whatever `x` is, which is what
/// makes the over-alignment idiom `iadd(sa, isub(K, band(sa, K - 1)))` analysable.
fn const_upper_bound(func: &ir::Function, v: Value, depth: u32) -> Option<u64> {
    ADDR_CACHE.with(|c| bound_rec(func, &mut c.borrow_mut(), v, depth))
}

fn bound_rec(
    func: &ir::Function,
    cache: &mut AddrCache,
    v: Value,
    depth: u32,
) -> Option<u64> {
    if depth == 0 {
        return None;
    }
    let dfg = &func.dfg;
    let v = dfg.resolve_aliases(v);
    if let Some(&hit) = cache.bound.get(&v) {
        return hit;
    }
    let result = match dfg.value_def(v) {
        ir::ValueDef::Result(inst, 0) => match &dfg.insts[inst] {
            ir::InstructionData::UnaryImm { opcode: ir::Opcode::Iconst, imm, .. } => {
                Some(imm.bits() as u64)
            }
            ir::InstructionData::Binary { opcode, .. } => {
                let args = dfg.insts[inst].arguments(&dfg.value_lists);
                let a = *args.first()?;
                let b = *args.get(1)?;
                let ba = bound_rec(func, cache, a, depth - 1);
                let bb = bound_rec(func, cache, b, depth - 1);
                match opcode {
                    ir::Opcode::Iadd => ba.zip(bb).and_then(|(x, y)| x.checked_add(y)),
                    ir::Opcode::Imul => ba.zip(bb).and_then(|(x, y)| x.checked_mul(y)),
                    // `x & M <= M` and `x & M <= x`, so a bound on either side
                    // bounds the result.
                    ir::Opcode::Band => match (ba, bb) {
                        (Some(x), Some(y)) => Some(x.min(y)),
                        (Some(x), None) | (None, Some(x)) => Some(x),
                        _ => None,
                    },
                    // `a - b <= a` because `b` is a non-negative integer here.
                    ir::Opcode::Isub => ba,
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    };
    cache.bound.insert(v, result);
    result
}

/// The stack slot `v` provably points into, plus an upper bound on how far past
/// the slot base it may point.
fn confined_slot(func: &ir::Function, v: Value, depth: u32) -> Option<(StackSlot, u64)> {
    if depth == 0 {
        return None;
    }
    let dfg = &func.dfg;
    let v = dfg.resolve_aliases(v);
    let ir::ValueDef::Result(inst, 0) = dfg.value_def(v) else { return None };
    let data = &dfg.insts[inst];
    match data {
        ir::InstructionData::StackAddr { stack_slot, offset, .. } => {
            Some((*stack_slot, i64::from(*offset) as u64))
        }
        ir::InstructionData::Binary { opcode: ir::Opcode::Iadd, .. } => {
            let args = data.arguments(&dfg.value_lists);
            let a = *args.first()?;
            let b = *args.get(1)?;
            if let Some((slot, off)) = confined_slot(func, a, depth - 1) {
                return Some((slot, off.checked_add(const_upper_bound(func, b, 8)?)?));
            }
            let (slot, off) = confined_slot(func, b, depth - 1)?;
            Some((slot, off.checked_add(const_upper_bound(func, a, 8)?)?))
        }
        _ => None,
    }
}

pub fn run(
    func: &mut ir::Function,
    endianness: ir::Endianness,
    analyses: &mut crate::analyses::Analyses,
) {
    if !crate::tunables::store_forward_enabled() {
        return;
    }
    run_body(func, endianness, analyses);
}

fn run_body(
    func: &mut ir::Function,
    endianness: ir::Endianness,
    analyses: &mut crate::analyses::Analyses,
) {
    ADDR_CACHE.with(|c| c.borrow_mut().bound.clear());

    let dbg = crate::tunables::store_forward_time() || crate::tunables::store_forward_debug();
    let t0 = std::time::Instant::now();
    if let Some(skip) = crate::tunables::store_forward_skip() {
        if func.name.to_string().contains(skip) {
            return;
        }
    }
    // A function with no stack slot has no aggregate round-trip to forward and no
    // dead store to drop: both halves of the pass key on a slot. Bailing before
    // `analyze` skips a full instruction walk over every such function, which on
    // a large crate is most of them (8843 functions, ~45 us each).
    if func.sized_stack_slots.is_empty() && func.dynamic_stack_slots.is_empty() {
        return;
    }
    let Some(Bases { private_slots: private, ptr_bases, loaded, confined, prov }) = analyze(func)
    else {
        // A truncated walk leaves the function untouched: correct, but the
        // optimization silently stops applying, so say so.
        if crate::tunables::store_forward_debug() {
            eprintln!("[sf] {} provenance walk truncated", func.name);
        }
        return;
    };
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
        cross_block_entry(func, &private, &ptr_bases, &loaded, &confined, &prov, analyses)
    } else {
        FxHashMap::default()
    };
    let t2 = std::time::Instant::now();
    let mut cross_forwarded = 0;
    if !loaded.is_empty() {
        for block in func.layout.blocks().collect::<Vec<_>>() {
            cross_forwarded += forward_block(
                func, block, &private, &ptr_bases, &loaded, &entry, endianness, &confined,
                &prov,
            );
        }
    }
    let t3 = std::time::Instant::now();
    drop_dead_stores(func, &private, &prov);
    if dbg {
        eprintln!(
            "[sf] {} insts={} setup={:?} cbe={:?} xblk={} fwd={:?} dead={:?}",
            func.name,
            func.dfg.num_insts(),
            t1 - t0,
            t2 - t1,
            cross_forwarded,
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
        if let ir::InstructionData::StackAddr { stack_slot, offset, .. } = func.dfg.insts[def_inst]
        {
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
    /// For each tracked pointer base, the slot it provably points into.
    confined: FxHashMap<Value, Confined>,
    /// The slot each tracked access base addresses: a direct `stack_addr` names
    /// its slot, a derived pointer names the slot it was computed from.
    prov: FxHashMap<Base, StackSlot>,
}

/// `None` when a provenance walk was truncated: no slot may then be assumed
/// private, so the pass leaves the function alone.
fn analyze(func: &ir::Function) -> Option<Bases> {
    let dfg = &func.dfg;
    let mut escaping_slots: FxHashSet<StackSlot> = FxHashSet::default();
    let mut slots: FxHashSet<StackSlot> = FxHashSet::default();
    let mut addr_only: FxHashSet<Value> = FxHashSet::default();
    let mut escaping_ptr: FxHashSet<Value> = FxHashSet::default();
    // Candidate loads; filtered against the final private sets below.
    let mut loads: Vec<(Inst, Value)> = Vec::new();
    let mut stores: Vec<(Inst, Value)> = Vec::new();

    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            let opcode = dfg.insts[inst].opcode();
            let addr_index = address_index(opcode);
            for (i, &arg) in dfg.insts[inst].arguments(&dfg.value_lists).iter().enumerate() {
                // Resolve once: the provenance walk and the pointer
                // classification both need the alias-resolved value, and
                // resolving twice dominated the walk.
                let arg = dfg.resolve_aliases(arg);
                let is_addr = addr_index == Some(i);
                // Every slot the argument may derive from: `low + addr(b)` names
                // both, and a call can read either.
                let complete = for_each_provenance_slot(func, arg, |slot| {
                    slots.insert(slot);
                    // Integer arithmetic on an address does not leak it out of
                    // the function, so only a *non-arithmetic* use other than a
                    // load/store address makes the slot observable elsewhere.
                    if !is_addr && !pure_address_arith(opcode) {
                        escaping_slots.insert(slot);
                    }
                });
                if !complete {
                    return None;
                }
                if is_addr {
                    addr_only.insert(arg);
                } else {
                    escaping_ptr.insert(arg);
                }
                if opcode.can_store() && i == 1 {
                    stores.push((inst, arg));
                }
            }
            // Branch arguments are *not* part of `arguments()`. An address passed
            // through a block parameter (a phi) is indistinguishable from an
            // escaping address by this pass, so treat it as one: otherwise a load
            // that reads the slot through the parameter is invisible and the
            // store it depends on is wrongly deleted.
            for dest in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables)
            {
                for arg in dest.args(&dfg.value_lists) {
                    if let Some(v) = arg.as_value() {
                        let v = dfg.resolve_aliases(v);
                        // Provenance, not just a direct `stack_addr`: a derived
                        // pointer reaching a phi is invisible to `mem_key` from
                        // then on, so the slot must not be treated as private.
                        if !for_each_provenance_slot(func, v, |slot| {
                            slots.insert(slot);
                            escaping_slots.insert(slot);
                        }) {
                            return None;
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

    // A pointer that is also an operand of another address computation is not a
    // tracked base (`record_store` treats its stores as untracked), so the
    // tracked-access test below must see the *final* pointer set. Testing the
    // candidate set accepted such a store as tracked, which kept the slot private
    // while forwarding could not invalidate it: a later direct load of the slot
    // then returned the value stored before it.
    addr_only.retain(|v| !escaping_ptr.contains(v));

    // An access whose address is derived from a slot but that the pass does not
    // track reads or writes that slot without a key: a dead-store decision based
    // on the tracked loads alone would then delete a store this load needs, and a
    // store it cannot see would make a forwarded fact stale. Such a slot is not
    // private.
    for (inst, addr) in stores.iter().chain(loads.iter()) {
        let tracked_access = mem_key(func, *inst, *addr)
            .map(|key| tracked(&key, &slots, &addr_only))
            .unwrap_or(false);
        if !tracked_access
            && !for_each_provenance_slot(func, *addr, |slot| {
                escaping_slots.insert(slot);
            })
        {
            return None;
        }
    }

    slots.retain(|s| !escaping_slots.contains(s));

    let mut loaded: FxHashSet<Key> = FxHashSet::default();
    for (inst, addr) in loads {
        if let Some(key) = mem_key(func, inst, addr) {
            if tracked(&key, &slots, &addr_only) {
                loaded.insert(key);
            }
        }
    }

    // Only worth computing when a pointer base survived: a private-slot store
    // never invalidates another slot's facts, so the alias question only comes up
    // between derived pointers.
    let mut confined: FxHashMap<Value, Confined> = FxHashMap::default();
    for &base in &addr_only {
        let Some((slot, bound)) = confined_slot(func, base, 8) else { continue };
        let Some(data) = func.sized_stack_slots.get(slot) else { continue };
        let size = u64::from(data.size);
        // A bound that already leaves the slot proves nothing about neighbours.
        if bound <= size {
            confined.insert(base, Confined { slot, bound, size });
        }
    }

    // Access bases, grouped by the slot they address. A slot reached through
    // several bases is still one slot: two bases may alias, and their offsets are
    // not comparable.
    let mut prov: FxHashMap<Base, StackSlot> = FxHashMap::default();
    for &slot in &slots {
        prov.insert(Base::Slot(slot), slot);
    }
    for &p in &addr_only {
        if let Some((slot, _)) = confined_slot(func, p, 8) {
            prov.insert(Base::Ptr(p), slot);
        }
    }

    Some(Bases { private_slots: slots, ptr_bases: addr_only, loaded, confined, prov })
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
    /// A fence: drops every fact, including stack slots, so a store is never
    /// forwarded past the ordering point (matches `forward_block`).
    Fence,
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
        if opcode == ir::Opcode::Fence {
            events.push(SfEvent::Fence);
        } else if clobbers_memory(opcode) {
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
/// running.
///
/// The dataflow is a single reverse-postorder sweep whose cost is linear in the
/// instruction count, measured at ~120 ns per instruction. The old 300-instruction
/// cutoff was set before that was measured, and it excluded the largest functions
/// even though they are exactly where a store in one block feeds a load in the
/// next. A hot loop whose trip-count pointer is materialised in one block and
/// tested in another kept a store/load round trip on the carried dependency chain
/// as a result: on the image pipeline that cost 5% of runtime.
///
/// The cap is now high enough to cover every function rakit emits for the
/// benchmarks (the largest is ~6.4k instructions, ~3 ms of dataflow) while still
/// bounding a pathological input.
fn cbe_within_budget(func: &ir::Function) -> bool {
    func.dfg.num_insts() <= crate::tunables::store_forward_cbe_max()
}

/// Largest function (in RAKIT instructions) the cross-block dataflow runs on.
pub const CBE_MAX_INSTS: usize = 16_384;

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
    confined: &FxHashMap<Value, Confined>,
    prov: &FxHashMap<Base, StackSlot>,
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
            return end <= o || prev_end <= start;
        }
        // Different bases. The fact survives only when the two provably address
        // different stack slots and neither access can leave its own slot; two
        // bases of the same slot may alias, and their offsets are relative to
        // different displacements so they cannot be compared.
        match (prov.get(&base), prov.get(&b)) {
            (Some(sb), Some(sf)) if sb != sf => {
                let inside = |base: Base, off: i64, ty: types::Type| match base {
                    Base::Slot(_) => true,
                    Base::Ptr(p) => confined.get(&p).is_some_and(|c| c.covers(off, ty)),
                };
                inside(base, off, ty) && inside(b, o, prev_ty)
            }
            _ => false,
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
    let ir::InstructionData::StackAddr { stack_slot, offset, .. } = func.dfg.insts[def_inst] else {
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
    confined: &FxHashMap<Value, Confined>,
    prov: &FxHashMap<Base, StackSlot>,
) -> usize {
    let mut avail: Avail = entry.get(&block).cloned().unwrap_or_default();
    // Facts inherited from the block entry are the cross-block ones; counting the
    // loads they forward is the only way to tell that path is exercised at all.
    let inherited = avail.clone();
    let mut from_entry = 0;
    // Set by a fence: the rest of the block may hold volatile accesses, which
    // this pass must neither forward into nor make forwardable.
    let mut sealed = false;

    for inst in func.layout.block_insts(block).collect::<Vec<_>>() {
        let opcode = func.dfg.insts[inst].opcode();

        if clobbers_memory(opcode) {
            if opcode == ir::Opcode::Fence {
                // A fence orders the accesses around it, and `volatile_load`
                // lowers to fence + trapping loads: a value stored before it
                // must not be forwarded into a load after it. Stores after it
                // may be volatile writes, so they must not become facts either:
                // forwarding their only reader would delete the write.
                avail.clear();
                sealed = true;
            } else {
                avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
            }
            continue;
        }
        if sealed {
            continue;
        }
        if opcode.can_store() {
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists);
            if args.len() >= 2 {
                let (value, addr) = (args[0], args[1]);
                record_store(
                    func, inst, value, addr, &mut avail, private, ptr_bases, loaded, confined,
                    prov,
                );
            }
        } else if opcode.can_load() {
            // A trapping load is the volatile read `volatile_load` emits;
            // forwarding a private stack store into it would delete the read.
            let flags = match &func.dfg.insts[inst] {
                ir::InstructionData::Load { flags, .. } => *flags,
                _ => continue,
            };
            if !func.dfg.mem_flags[flags].notrap() {
                continue;
            }
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
                if inherited.get(&key) == Some(&(value, ty)) {
                    from_entry += 1;
                }
            } else if result_ty.bytes() == ty.bytes() && result_ty.is_vector() && ty.is_vector() {
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
                if inherited.get(&key) == Some(&(value, ty)) {
                    from_entry += 1;
                }
            }
        }
    }
    from_entry
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
    confined: &FxHashMap<Value, Confined>,
    prov: &FxHashMap<Base, StackSlot>,
) -> Avail {
    // A fence seals the block exactly as it does in `forward_block`: the stores
    // after it may be volatile writes, so they must not become facts a successor
    // forwards into. Recording them here while the block-local walk refuses them
    // made the two halves of the pass disagree about the same store.
    let mut sealed = false;
    for event in events {
        match *event {
            SfEvent::Clobber => {
                avail.retain(|&(b, _), _| matches!(b, Base::Slot(_)));
            }
            SfEvent::Fence => {
                avail.clear();
                sealed = true;
            }
            SfEvent::Store { inst, value, addr } => {
                if sealed {
                    continue;
                }
                record_store(
                    func, inst, value, addr, &mut avail, private, ptr_bases, loaded, confined,
                    prov,
                );
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
    confined: &FxHashMap<Value, Confined>,
    prov: &FxHashMap<Base, StackSlot>,
    analyses: &mut crate::analyses::Analyses,
) -> FxHashMap<Block, Avail> {
    // Timing is only collected when explicitly requested: `Instant::now()` in a
    // per-block loop is measurable compile time on its own.
    let timed = crate::tunables::store_forward_time();
    let now = |on: bool| on.then(std::time::Instant::now);
    let elapsed_ns = |t: Option<std::time::Instant>| t.map_or(0, |t| t.elapsed().as_nanos());

    let t_cfg = now(timed);
    let cfg = analyses.cfg(func);
    let blocks: Vec<Block> = func.layout.blocks().collect();
    let cfg_ns = elapsed_ns(t_cfg);
    // Decode each block's store events once; the dataflow below revisits blocks.
    let t_ev = now(timed);
    let events: FxHashMap<Block, Vec<SfEvent>> =
        blocks.iter().map(|&b| (b, block_events(func, b))).collect();
    let ev_ns = elapsed_ns(t_ev);
    // The entry block is also reached from outside the function, where no store
    // has run. The CFG does not model that edge, so a self-loop on the entry
    // block would otherwise leak facts from the back edge into its own entry.
    let entry_block = func.layout.entry_block();

    // The meet is an intersection, so a single predecessor that contributes no
    // facts — an unreached back edge, or a path that stores nothing — collapses
    // the whole result to empty. Returning early in that case avoids cloning a
    // map for most blocks in a real function.
    let meet = |out: &FxHashMap<Block, Avail>, block: Block| -> Avail {
        if Some(block) == entry_block {
            return Avail::default();
        }
        let preds: Vec<Block> = cfg.pred_iter(block).map(|p| p.block).collect();
        let Some((first, rest)) = preds.split_first() else { return Avail::default() };
        let Some(first) = out.get(first).filter(|a| !a.is_empty()) else {
            return Avail::default();
        };
        let mut acc = first.clone();
        for pred in rest {
            let Some(po) = out.get(pred) else { return Avail::default() };
            acc.retain(|k, v| po.get(k) == Some(v));
            if acc.is_empty() {
                break;
            }
        }
        acc
    };

    // One reverse-postorder sweep. For a reducible CFG — everything rakit
    // emits — a block's predecessors other than back edges are visited before
    // it, so a single pass reaches the same facts as a fixed point. A back edge
    // is not yet available, so it contributes no facts: the result is a
    // conservative under-approximation, which is always safe for forwarding.
    let mut entry: FxHashMap<Block, Avail> = FxHashMap::default();
    let mut out: FxHashMap<Block, Avail> = FxHashMap::default();
    let mut meet_ns: u128 = 0;
    let mut trans_ns: u128 = 0;
    let mut max_avail: usize = 0;
    let mut meet_work: usize = 0;
    let t_loop = now(timed);
    for block in reverse_postorder(&cfg, entry_block, &blocks) {
        if Some(block) == entry_block {
            continue;
        }
        let t_m = now(timed);
        let in_avail = meet(&out, block);
        meet_ns += elapsed_ns(t_m);
        max_avail = max_avail.max(in_avail.len());
        meet_work += in_avail.len();
        entry.insert(block, in_avail.clone());
        let t_t = now(timed);
        let new_out =
            transfer(func, &events[&block], in_avail, private, ptr_bases, loaded, confined, prov);
        trans_ns += elapsed_ns(t_t);
        out.insert(block, new_out);
    }
    let loop_ns = elapsed_ns(t_loop);
    if timed {
        let nonempty = entry.values().filter(|a| !a.is_empty()).count();
        let facts: usize = entry.values().map(|a| a.len()).sum();
        eprintln!(
            "[cbe] {} blocks={} insts={} cfg={:.0}us events={:.0}us loop={:.0}us \
             meet={:.0}us transfer={:.0}us maxavail={} avail_sum={} \
             nonempty_blocks={} total_facts={}",
            func.name,
            blocks.len(),
            func.dfg.num_insts(),
            cfg_ns as f64 / 1000.0,
            ev_ns as f64 / 1000.0,
            loop_ns as f64 / 1000.0,
            meet_ns as f64 / 1000.0,
            trans_ns as f64 / 1000.0,
            max_avail,
            meet_work,
            nonempty,
            facts,
        );
    }
    entry
}

/// Blocks in reverse postorder, reachable from the entry block. Blocks that are
/// not reachable keep their default (empty) entry facts.
///
/// `Block` is a dense entity index, so the successor lists live in a `Vec`
/// indexed by it rather than a map: the DFS revisits this table once per edge,
/// and hashing a `Block` on every step was a measurable share of the sweep.
fn reverse_postorder(
    cfg: &rakit_codegen::flowgraph::ControlFlowGraph,
    entry: Option<Block>,
    blocks: &[Block],
) -> Vec<Block> {
    let Some(entry) = entry else { return blocks.to_vec() };
    let num_blocks = blocks.iter().map(|b| b.as_u32() as usize + 1).max().unwrap_or(0);
    let mut succs: Vec<Vec<Block>> = vec![Vec::new(); num_blocks];
    for &b in blocks {
        succs[b.as_u32() as usize] = cfg.succ_iter(b).collect();
    }
    let mut visited = vec![false; num_blocks];
    let mut postorder: Vec<Block> = Vec::with_capacity(blocks.len());
    // Iterative DFS; the `(block, next-succ)` stack mirrors the recursion.
    let mut stack: Vec<(Block, usize)> = vec![(entry, 0)];
    visited[entry.as_u32() as usize] = true;
    while let Some(&mut (block, ref mut i)) = stack.last_mut() {
        let next = succs[block.as_u32() as usize].get(*i).copied();
        match next {
            Some(succ) => {
                *i += 1;
                let s = succ.as_u32() as usize;
                if !visited[s] {
                    visited[s] = true;
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
fn drop_dead_stores(
    func: &mut ir::Function,
    private: &FxHashSet<StackSlot>,
    prov: &FxHashMap<Base, StackSlot>,
) {
    // Every store this pass can prove dead targets a private slot, so with none
    // the scan below would find nothing to remove.
    if private.is_empty() {
        return;
    }
    // Reads bucketed by slot: a store only has to be compared against the reads
    // of its own slot, not against every load in the function.
    let mut reads: FxHashMap<StackSlot, Vec<(i64, i64)>> = FxHashMap::default();
    // Slots read through a derived pointer. Those offsets are relative to a
    // displacement this pass does not know, so they cannot be compared with a
    // store's offset; the slot counts as read as a whole instead.
    let mut ptr_reads: FxHashSet<StackSlot> = FxHashSet::default();
    for block in func.layout.blocks() {
        for inst in func.layout.block_insts(block) {
            if !func.dfg.insts[inst].opcode().can_load() {
                continue;
            }
            let addr = func.dfg.insts[inst].arguments(&func.dfg.value_lists).first().copied();
            let Some(addr) = addr else { continue };
            let Some(key) = stack_key(func, inst, addr) else {
                if !for_each_provenance_slot(func, addr, |slot| {
                    ptr_reads.insert(slot);
                }) {
                    // An incomplete walk cannot say which slot this read names,
                    // so no store of this function may be called dead.
                    ptr_reads.extend(private.iter().copied());
                }
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
        // Rakit has no volatile bit: `volatile_store` lowers to `fence` plus
        // ordinary stores, so after a fence a store may be an observable write
        // and must survive even with no reader.
        let mut saw_fence = false;
        for inst in func.layout.block_insts(block) {
            let opcode = func.dfg.insts[inst].opcode();
            if opcode == ir::Opcode::Fence {
                saw_fence = true;
                continue;
            }
            if saw_fence || !opcode.can_store() {
                continue;
            }
            let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists);
            let Some(addr) = args.get(1).copied() else { continue };
            let Some(key) = mem_key(func, inst, addr) else { continue };
            let Some(&slot) = prov.get(&key.0) else { continue };
            if !private.contains(&slot) {
                continue;
            }
            let size = i64::from(func.dfg.value_type(args[0]).bytes());
            let overlaps = match key.0 {
                // A direct slot access: compare byte ranges.
                Base::Slot(_) => {
                    ptr_reads.contains(&slot)
                        || reads.get(&slot).is_some_and(|rs| {
                            rs.iter().any(|&(off, len)| off < key.1 + size && key.1 < off + len)
                        })
                }
                // A derived pointer: its offset is relative to an unknown
                // displacement, so any read of the slot keeps the store alive.
                Base::Ptr(p) => {
                    let confined = confined_slot(func, p, 8);
                    ptr_reads.contains(&slot)
                        || reads.contains_key(&slot)
                        || confined.is_none()
                }
            };
            if !overlaps {
                dead.push(inst);
            }
        }
    }
    for inst in dead {
        func.layout.remove_inst(inst);
    }
}
