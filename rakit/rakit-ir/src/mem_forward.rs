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
enum Key {
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

fn size_of(ty: Type) -> u32 {
    match ty {
        Type::I8 | Type::Bool => 1,
        Type::I16 => 2,
        Type::I32 | Type::F32 => 4,
        Type::I64 | Type::F64 | Type::Ptr => 8,
    }
}

fn definitions(func: &Function) -> HashMap<Value, Inst> {
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

fn escaping_slots(func: &Function, defs: &HashMap<Value, Inst>) -> Vec<bool> {
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

fn key_of(
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

fn dse(func: &mut Function) -> bool {
    let defs = definitions(func);
    let escaping = escaping_slots(func, &defs);
    let mut loads: HashMap<u32, Vec<(i64, i64)>> = HashMap::new();
    let mut dynamic_load = vec![false; func.stack_slots.len()];

    for blk in &func.blocks {
        for inst in &blk.insts {
            match inst {
                Inst::Load(_, ty, addr, offset) => {
                    let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    let slot = key.slot();
                    if escaping[slot as usize] {
                        return false;
                    }
                    match key {
                        Key::Const { offset, .. } => {
                            loads
                                .entry(slot)
                                .or_default()
                                .push((offset, offset + size_of(*ty) as i64));
                        }
                        Key::Dyn { .. } => dynamic_load[slot as usize] = true,
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

    let mut changed = false;
    for bi in 0..func.blocks.len() {
        let mut ii = 0;
        while ii < func.blocks[bi].insts.len() {
            let dead = match &func.blocks[bi].insts[ii] {
                Inst::Store(ty, addr, offset, _) => {
                    let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    let slot = key.slot();
                    match key {
                        Key::Const { offset, .. } => {
                            if dynamic_load[slot as usize] {
                                false
                            } else {
                                let end = offset + size_of(*ty) as i64;
                                !loads
                                    .get(&slot)
                                    .map(|ranges| {
                                        ranges.iter().any(|&(a, b)| a < end && offset < b)
                                    })
                                    .unwrap_or(false)
                            }
                        }
                        Key::Dyn { .. } => {
                            !loads.get(&slot).map(|r| !r.is_empty()).unwrap_or(false)
                                && !dynamic_load[slot as usize]
                        }
                    }
                }
                _ => false,
            };
            if dead {
                func.blocks[bi].insts.remove(ii);
                changed = true;
            } else {
                ii += 1;
            }
        }
    }
    changed
}
