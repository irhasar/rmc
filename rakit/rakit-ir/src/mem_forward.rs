//! Forward stores to later loads of the same private stack address and remove
//! stores whose bytes are never loaded.
//!
//! Addresses are tracked through `StackAddr`, `Copy`, constant `Add`, and
//! `Select`. A non-constant index produces a `Dyn` key that is matched only
//! against the same index expression, so `a[i]` store/load pairs can be
//! forwarded without assuming anything about `i`. Any dynamic store kills all
//! facts for its slot, and any dynamic load suppresses dead-store removal for
//! that slot.

use std::collections::{BTreeMap, HashMap};

use crate::cfg_util::{predecessors, rpo};
use crate::ir::{BinOp, Function, Inst, Type, Value};

type Facts = BTreeMap<Key, (Value, Type)>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Key {
    Const { slot: u32, offset: i64 },
    Dyn { slot: u32, base: Value, offset: i64 },
}

impl Key {
    fn slot(self) -> u32 {
        match self {
            Key::Const { slot, .. } | Key::Dyn { slot, .. } => slot,
        }
    }
}

pub fn run(func: &mut Function) -> bool {
    let mut changed = false;
    for _ in 0..8 {
        let forwarded = forward(func);
        let removed = dse(func);
        if !forwarded && !removed {
            break;
        }
        changed = true;
    }
    changed
}

pub(crate) fn size_of(ty: Type) -> u32 {
    match ty {
        Type::I8 | Type::Bool => 1,
        Type::I16 => 2,
        Type::I32 | Type::F32 => 4,
        Type::I64 | Type::F64 | Type::Ptr => 8,
    }
}

pub(crate) fn definitions(func: &Function) -> HashMap<Value, Inst> {
    let mut defs = HashMap::new();
    for blk in &func.blocks {
        for inst in &blk.insts {
            if let Some(r) = inst.result_value() {
                defs.insert(r, inst.clone());
            }
        }
    }
    defs
}

fn const_of(defs: &HashMap<Value, Inst>, value: Value) -> Option<i64> {
    match defs.get(&value)? {
        Inst::Iconst(_, ty, v) if ty.is_int() => Some(*v),
        Inst::Copy(_, _, x) => const_of(defs, *x),
        _ => None,
    }
}

fn address_expr(defs: &HashMap<Value, Inst>, value: Value, depth: u32) -> Option<Key> {
    if depth > 32 {
        return None;
    }
    match defs.get(&value)? {
        Inst::StackAddr(_, slot, offset) => Some(Key::Const {
            slot: slot.0,
            offset: *offset as i64,
        }),
        Inst::Copy(_, _, x) => address_expr(defs, *x, depth + 1),
        Inst::Bin(_, BinOp::Add, _, x, y) => add_expr(defs, value, *x, *y, depth + 1),
        Inst::Select(_, _, _, x, y) => {
            let ex = address_expr(defs, *x, depth + 1)?;
            let ey = address_expr(defs, *y, depth + 1)?;
            if ex.slot() == ey.slot() {
                Some(Key::Dyn {
                    slot: ex.slot(),
                    base: value,
                    offset: 0,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

fn add_expr(
    defs: &HashMap<Value, Inst>,
    value: Value,
    x: Value,
    y: Value,
    depth: u32,
) -> Option<Key> {
    let ex = address_expr(defs, x, depth);
    let ey = address_expr(defs, y, depth);
    let cx = const_of(defs, x);
    let cy = const_of(defs, y);
    match (ex, ey, cx, cy) {
        (
            Some(Key::Const { slot: s1, offset: o1 }),
            Some(Key::Const { slot: s2, offset: o2 }),
            _,
            _,
        ) if s1 == s2 => Some(Key::Const {
            slot: s1,
            offset: o1 + o2,
        }),
        (Some(Key::Const { slot, offset }), None, _, Some(c)) => Some(Key::Const {
            slot,
            offset: offset + c,
        }),
        (None, Some(Key::Const { slot, offset }), Some(c), _) => Some(Key::Const {
            slot,
            offset: offset + c,
        }),
        (Some(Key::Const { slot, offset }), None, _, None) => Some(Key::Dyn {
            slot,
            base: y,
            offset,
        }),
        (None, Some(Key::Const { slot, offset }), None, _) => Some(Key::Dyn {
            slot,
            base: x,
            offset,
        }),
        (Some(Key::Dyn { slot, base, offset }), None, _, Some(c)) => Some(Key::Dyn {
            slot,
            base,
            offset: offset + c,
        }),
        (None, Some(Key::Dyn { slot, base, offset }), Some(c), _) => Some(Key::Dyn {
            slot,
            base,
            offset: offset + c,
        }),
        (Some(a), Some(b), _, _) if a.slot() == b.slot() => Some(Key::Dyn {
            slot: a.slot(),
            base: value,
            offset: 0,
        }),
        _ => None,
    }
}

pub(crate) fn escaping_slots(func: &Function, defs: &HashMap<Value, Inst>) -> Vec<bool> {
    let mut escaping = vec![false; func.stack_slots.len()];
    for blk in &func.blocks {
        for inst in &blk.insts {
            let result_addr = inst.result_value().and_then(|r| address_expr(defs, r, 0));
            for u in inst.operands() {
                let Some(addr) = address_expr(defs, u, 0) else {
                    continue;
                };
                let address_use = match inst {
                    Inst::Load(_, _, a, _) => *a == u,
                    Inst::Store(_, a, _, _) => *a == u,
                    Inst::Copy(_, _, _) => result_addr.is_some(),
                    Inst::Bin(_, BinOp::Add, _, _, _) => result_addr.is_some(),
                    Inst::Select(_, _, _, _, _) => result_addr.is_some(),
                    _ => false,
                };
                if !address_use {
                    if let Some(e) = escaping.get_mut(addr.slot() as usize) {
                        *e = true;
                    }
                }
            }
        }
    }
    escaping
}

pub(crate) fn key_of(
    defs: &HashMap<Value, Inst>,
    func: &Function,
    addr: Value,
    offset: i32,
    ty: Type,
) -> Option<Key> {
    match address_expr(defs, addr, 0)? {
        Key::Const { slot, offset: base } => {
            let data = func.stack_slots.get(slot as usize)?;
            let offset = base + offset as i64;
            let size = size_of(ty) as i64;
            if offset < 0 || offset.checked_add(size)? > data.size as i64 {
                return None;
            }
            Some(Key::Const { slot, offset })
        }
        Key::Dyn {
            slot,
            base,
            offset: base_offset,
        } => Some(Key::Dyn {
            slot,
            base,
            offset: base_offset + offset as i64,
        }),
    }
}

fn kill(facts: &mut Facts, key: Key, ty: Type) {
    let slot = key.slot();
    match key {
        Key::Const { offset, .. } => {
            let size = size_of(ty) as i64;
            facts.retain(|k, (_, t)| {
                if k.slot() != slot {
                    return true;
                }
                match *k {
                    Key::Dyn { .. } => false,
                    Key::Const { offset: old, .. } => {
                        let old_size = size_of(*t) as i64;
                        !(old < offset + size && offset < old + old_size)
                    }
                }
            });
        }
        Key::Dyn { .. } => {
            facts.retain(|k, _| k.slot() != slot);
        }
    }
}

fn forward(func: &mut Function) -> bool {
    let defs = definitions(func);
    let escaping = escaping_slots(func, &defs);

    for blk in &func.blocks {
        for inst in &blk.insts {
            if let Inst::Store(ty, addr, offset, _) = inst {
                let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                    return false;
                };
                if escaping[key.slot() as usize] {
                    return false;
                }
            }
        }
    }

    let preds = predecessors(func);
    let order = rpo(func);
    let mut out: Vec<Option<Facts>> = vec![None; func.blocks.len()];
    let mut replacements = Vec::new();

    for &bi in &order {
        let mut facts = Facts::new();
        let mut first = true;
        for &p in &preds[bi] {
            let Some(pf) = &out[p] else {
                continue;
            };
            if first {
                facts = pf.clone();
                first = false;
            } else {
                facts.retain(|k, v| pf.get(k) == Some(v));
            }
        }
        if bi == func.entry.0 as usize {
            facts.clear();
        }

        for (ii, inst) in func.blocks[bi].insts.iter().enumerate() {
            match inst {
                Inst::Load(r, ty, addr, offset) => {
                    if let Some(key) = key_of(&defs, func, *addr, *offset, *ty) {
                        if let Some(&(value, value_ty)) = facts.get(&key) {
                            if value_ty == *ty {
                                replacements.push((bi, ii, *r, *ty, value));
                            }
                        }
                    }
                }
                Inst::Store(ty, addr, offset, value) => {
                    if let Some(key) = key_of(&defs, func, *addr, *offset, *ty) {
                        kill(&mut facts, key, *ty);
                        facts.insert(key, (*value, *ty));
                    }
                }
                _ => {}
            }
        }
        out[bi] = Some(facts);
    }

    let mut changed = false;
    for (bi, ii, r, ty, value) in replacements {
        if matches!(func.blocks[bi].insts.get(ii), Some(Inst::Load(..))) {
            func.blocks[bi].insts[ii] = Inst::Copy(r, ty, value);
            changed = true;
        }
    }
    changed
}

type SlotRanges = BTreeMap<u32, Vec<(i64, i64)>>;

fn add_range(ranges: &mut Vec<(i64, i64)>, start: i64, end: i64) {
    if start >= end {
        return;
    }
    ranges.push((start, end));
    ranges.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(ranges.len());
    for &(s, e) in ranges.iter() {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                if e > last.1 {
                    last.1 = e;
                }
                continue;
            }
        }
        merged.push((s, e));
    }
    *ranges = merged;
}

fn add_slot(map: &mut SlotRanges, slot: u32, start: i64, end: i64) {
    add_range(map.entry(slot).or_default(), start, end);
}

fn sub_range(ranges: &[(i64, i64)], start: i64, end: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    for &(s, e) in ranges {
        if e <= start || s >= end {
            out.push((s, e));
        } else {
            if s < start {
                out.push((s, start));
            }
            if e > end {
                out.push((end, e));
            }
        }
    }
    out
}

fn sub_slot(map: &mut SlotRanges, slot: u32, start: i64, end: i64) {
    if let Some(ranges) = map.get_mut(&slot) {
        *ranges = sub_range(ranges, start, end);
        if ranges.is_empty() {
            map.remove(&slot);
        }
    }
}

fn union(a: &SlotRanges, b: &SlotRanges) -> SlotRanges {
    let mut out = a.clone();
    for (&slot, ranges) in b {
        for &(s, e) in ranges {
            add_slot(&mut out, slot, s, e);
        }
    }
    out
}

fn subtract(a: &SlotRanges, b: &SlotRanges) -> SlotRanges {
    let mut out = a.clone();
    for (&slot, ranges) in b {
        for &(s, e) in ranges {
            sub_slot(&mut out, slot, s, e);
        }
    }
    out
}

fn intersects(ranges: &[(i64, i64)], start: i64, end: i64) -> bool {
    ranges.iter().any(|&(s, e)| s < end && start < e)
}

fn whole_slot(func: &Function, slot: u32) -> (i64, i64) {
    (0, func.stack_slots[slot as usize].size as i64)
}

fn key_range(func: &Function, key: Key, ty: Type) -> (i64, i64) {
    match key {
        Key::Const { offset, .. } => (offset, offset + size_of(ty) as i64),
        Key::Dyn { slot, .. } => whole_slot(func, slot),
    }
}

struct Effects {
    used: SlotRanges,
    defined: SlotRanges,
}

fn block_effects(func: &Function, defs: &HashMap<Value, Inst>, bi: usize) -> Effects {
    let mut used = SlotRanges::new();
    let mut defined = SlotRanges::new();
    for inst in &func.blocks[bi].insts {
        match inst {
            Inst::Load(_, ty, addr, offset) => {
                if let Some(key) = key_of(defs, func, *addr, *offset, *ty) {
                    let slot = key.slot();
                    let (start, end) = key_range(func, key, *ty);
                    let exposed = match defined.get(&slot) {
                        Some(killed) => {
                            let mut ranges = vec![(start, end)];
                            for &(s, e) in killed {
                                ranges = sub_range(&ranges, s, e);
                            }
                            ranges
                        }
                        None => vec![(start, end)],
                    };
                    for (s, e) in exposed {
                        add_slot(&mut used, slot, s, e);
                    }
                }
            }
            Inst::Store(ty, addr, offset, _) => {
                if let Some(key) = key_of(defs, func, *addr, *offset, *ty) {
                    let (start, end) = key_range(func, key, *ty);
                    add_slot(&mut defined, key.slot(), start, end);
                }
            }
            _ => {}
        }
    }
    Effects { used, defined }
}

fn dse(func: &mut Function) -> bool {
    let defs = definitions(func);
    let escaping = escaping_slots(func, &defs);

    for blk in &func.blocks {
        for inst in &blk.insts {
            match inst {
                Inst::Load(_, ty, addr, offset) => {
                    let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    if escaping[key.slot() as usize] {
                        return false;
                    }
                }
                Inst::Store(ty, addr, offset, _) => {
                    let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    if escaping[key.slot() as usize] {
                        return false;
                    }
                }
                _ => {}
            }
        }
    }

    let preds = predecessors(func);
    let order = rpo(func);
    let mut succ = vec![Vec::new(); func.blocks.len()];
    for (b, ps) in preds.iter().enumerate() {
        for &p in ps {
            succ[p].push(b);
        }
    }

    let effects: Vec<Effects> = (0..func.blocks.len())
        .map(|bi| block_effects(func, &defs, bi))
        .collect();
    let mut live_in = vec![SlotRanges::new(); func.blocks.len()];
    let mut live_out = vec![SlotRanges::new(); func.blocks.len()];

    loop {
        let mut changed = false;
        for &bi in order.iter().rev() {
            let mut out = SlotRanges::new();
            for &s in &succ[bi] {
                out = union(&out, &live_in[s]);
            }
            let in_ = union(&effects[bi].used, &subtract(&out, &effects[bi].defined));
            if out != live_out[bi] || in_ != live_in[bi] {
                live_out[bi] = out;
                live_in[bi] = in_;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut changed = false;
    for bi in 0..func.blocks.len() {
        let mut live = live_out[bi].clone();
        let mut ii = func.blocks[bi].insts.len();
        while ii > 0 {
            ii -= 1;
            match &func.blocks[bi].insts[ii] {
                Inst::Load(_, ty, addr, offset) => {
                    if let Some(key) = key_of(&defs, func, *addr, *offset, *ty) {
                        let (start, end) = key_range(func, key, *ty);
                        add_slot(&mut live, key.slot(), start, end);
                    }
                }
                Inst::Store(ty, addr, offset, _) => {
                    if let Some(key) = key_of(&defs, func, *addr, *offset, *ty) {
                        let (start, end) = key_range(func, key, *ty);
                        let slot = key.slot();
                        let needed = live
                            .get(&slot)
                            .map(|ranges| intersects(ranges, start, end))
                            .unwrap_or(false);
                        if needed {
                            sub_slot(&mut live, slot, start, end);
                        } else {
                            func.blocks[bi].insts.remove(ii);
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    changed
}
