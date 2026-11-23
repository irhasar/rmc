//! Forward stores to later loads of the same private stack slot and remove
//! stores whose bytes are never loaded.
//!
//! The pass is deliberately conservative: it only tracks addresses that are
//! direct `StackAddr` values (possibly through `Copy`), requires every store to
//! resolve to a non-escaping slot, and skips the whole function if any address
//! escapes or any access is out of bounds.

use std::collections::{BTreeMap, HashMap};

use crate::cfg_util::{predecessors, rpo};
use crate::ir::{Function, Inst, Type, Value};

type Facts = BTreeMap<(u32, i64), (Value, Type)>;

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

fn address_map(func: &Function) -> HashMap<Value, (u32, i64)> {
    let mut map = HashMap::new();
    for blk in &func.blocks {
        for inst in &blk.insts {
            if let Inst::StackAddr(r, slot, offset) = inst {
                map.insert(*r, (slot.0, *offset as i64));
            }
        }
    }
    loop {
        let mut changed = false;
        for blk in &func.blocks {
            for inst in &blk.insts {
                if let Inst::Copy(r, _, x) = inst {
                    if let Some(k) = map.get(x).copied() {
                        if map.insert(*r, k).is_none() {
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    map
}

fn escaping_slots(func: &Function, addrs: &HashMap<Value, (u32, i64)>) -> Vec<bool> {
    let mut escaping = vec![false; func.stack_slots.len()];
    for blk in &func.blocks {
        for inst in &blk.insts {
            for u in inst.operands() {
                let Some(&(slot, _)) = addrs.get(&u) else {
                    continue;
                };
                let address_use = match inst {
                    Inst::Load(_, _, addr, _) => *addr == u,
                    Inst::Store(_, addr, _, _) => *addr == u,
                    Inst::Copy(r, _, _) => addrs.contains_key(r),
                    _ => false,
                };
                if !address_use {
                    if let Some(e) = escaping.get_mut(slot as usize) {
                        *e = true;
                    }
                }
            }
        }
    }
    escaping
}

fn key_of(
    addrs: &HashMap<Value, (u32, i64)>,
    func: &Function,
    addr: Value,
    offset: i32,
    ty: Type,
) -> Option<(u32, i64)> {
    let &(slot, base) = addrs.get(&addr)?;
    let data = func.stack_slots.get(slot as usize)?;
    let offset = base + offset as i64;
    let size = size_of(ty) as i64;
    if offset < 0 || offset.checked_add(size)? > data.size as i64 {
        return None;
    }
    Some((slot, offset))
}

fn forward(func: &mut Function) -> bool {
    let addrs = address_map(func);
    let escaping = escaping_slots(func, &addrs);

    for blk in &func.blocks {
        for inst in &blk.insts {
            if let Inst::Store(ty, addr, offset, _) = inst {
                let Some((slot, _)) = key_of(&addrs, func, *addr, *offset, *ty) else {
                    return false;
                };
                if escaping[slot as usize] {
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
                    if let Some(key) = key_of(&addrs, func, *addr, *offset, *ty) {
                        if let Some(&(value, value_ty)) = facts.get(&key) {
                            if value_ty == *ty {
                                replacements.push((bi, ii, *r, *ty, value));
                            }
                        }
                    }
                }
                Inst::Store(ty, addr, offset, value) => {
                    if let Some(key) = key_of(&addrs, func, *addr, *offset, *ty) {
                        let (slot, off) = key;
                        let size = size_of(*ty) as i64;
                        facts.retain(|&(s, o), (_, t)| {
                            if s != slot {
                                return true;
                            }
                            let old_size = size_of(*t) as i64;
                            !(o < off + size && off < o + old_size)
                        });
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
    let addrs = address_map(func);
    let escaping = escaping_slots(func, &addrs);
    let mut loads: HashMap<u32, Vec<(i64, i64)>> = HashMap::new();

    for blk in &func.blocks {
        for inst in &blk.insts {
            match inst {
                Inst::Load(_, ty, addr, offset) => {
                    let Some((slot, off)) = key_of(&addrs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    if escaping[slot as usize] {
                        return false;
                    }
                    loads
                        .entry(slot)
                        .or_default()
                        .push((off, off + size_of(*ty) as i64));
                }
                Inst::Store(ty, addr, offset, _) => {
                    let Some((slot, _)) = key_of(&addrs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    if escaping[slot as usize] {
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
                    let Some((slot, off)) = key_of(&addrs, func, *addr, *offset, *ty) else {
                        return false;
                    };
                    let end = off + size_of(*ty) as i64;
                    let live = loads
                        .get(&slot)
                        .map(|ranges| ranges.iter().any(|&(a, b)| a < end && off < b))
                        .unwrap_or(false);
                    !live
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
