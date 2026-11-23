//! Global value numbering: replace a repeated pure instruction with a copy of
//! the first result that dominates it. Expressions are inherited along the
//! dominator tree, so a value is reused only where it is available.

use std::collections::HashMap;

use crate::cfg_util::{dominators, predecessors, rpo};
use crate::ir::{BinOp, FloatCc, Function, Inst, IntCc, Type, Value};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Iconst(Type, i64),
    F32const(u32),
    F64const(u64),
    Copy(Type, Value),
    Bin(BinOp, Type, Value, Value),
    Icmp(IntCc, Type, Value, Value),
    Fcmp(FloatCc, Type, Value, Value),
    Select(Type, Value, Value, Value),
}

pub fn run(func: &mut Function) -> bool {
    let n = func.blocks.len();
    if n == 0 {
        return false;
    }
    let preds = predecessors(func);
    let order = rpo(func);
    let idom = dominators(func, &preds, &order);
    let mut in_order = vec![false; n];
    let mut tables: Vec<HashMap<Key, Value>> = vec![HashMap::new(); n];
    let mut changed = false;

    for &bi in &order {
        in_order[bi] = true;
        let mut table = match idom[bi] {
            Some(d) if d != bi => tables[d].clone(),
            _ => HashMap::new(),
        };
        let insts = std::mem::take(&mut func.blocks[bi].insts);
        let mut out = Vec::with_capacity(insts.len());
        for inst in insts {
            let Some(key) = key_of(&inst) else {
                out.push(inst);
                continue;
            };
            if let Some(&existing) = table.get(&key) {
                let r = inst.result_value().expect("keyed instruction has a result");
                let ty = func.values[r.0 as usize];
                out.push(Inst::Copy(r, ty, existing));
                changed = true;
            } else {
                if let Some(r) = inst.result_value() {
                    table.insert(key, r);
                }
                out.push(inst);
            }
        }
        func.blocks[bi].insts = out;
        tables[bi] = table;
    }

    for bi in 0..n {
        if in_order[bi] {
            continue;
        }
        let mut table: HashMap<Key, Value> = HashMap::new();
        let insts = std::mem::take(&mut func.blocks[bi].insts);
        let mut out = Vec::with_capacity(insts.len());
        for inst in insts {
            let Some(key) = key_of(&inst) else {
                out.push(inst);
                continue;
            };
            if let Some(&existing) = table.get(&key) {
                let r = inst.result_value().expect("keyed instruction has a result");
                let ty = func.values[r.0 as usize];
                out.push(Inst::Copy(r, ty, existing));
                changed = true;
            } else {
                if let Some(r) = inst.result_value() {
                    table.insert(key, r);
                }
                out.push(inst);
            }
        }
        func.blocks[bi].insts = out;
    }

    changed
}

fn key_of(inst: &Inst) -> Option<Key> {
    Some(match inst {
        Inst::Iconst(_, ty, v) => Key::Iconst(*ty, *v),
        Inst::F32const(_, v) => Key::F32const(v.to_bits()),
        Inst::F64const(_, v) => Key::F64const(v.to_bits()),
        Inst::Copy(_, ty, x) => Key::Copy(*ty, *x),
        Inst::Bin(_, op, ty, x, y) => Key::Bin(*op, *ty, *x, *y),
        Inst::Icmp(_, cc, ty, x, y) => Key::Icmp(*cc, *ty, *x, *y),
        Inst::Fcmp(_, cc, ty, x, y) => Key::Fcmp(*cc, *ty, *x, *y),
        Inst::Select(_, ty, c, x, y) => Key::Select(*ty, *c, *x, *y),
        _ => return None,
    })
}
