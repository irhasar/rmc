//! Global value numbering: replace a repeated pure instruction with a copy of
//! the first result that dominates it. Expressions are inherited along the
//! dominator tree, so a value is reused only where it is available.

use std::collections::HashMap;

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

fn predecessors(func: &Function) -> Vec<Vec<usize>> {
    let mut preds = vec![Vec::new(); func.blocks.len()];
    for (bi, block) in func.blocks.iter().enumerate() {
        let Some(term) = block.insts.last() else {
            continue;
        };
        match term {
            Inst::Jump(t, _) => preds[t.0 as usize].push(bi),
            Inst::Brif(_, t, _, f, _) => {
                preds[t.0 as usize].push(bi);
                preds[f.0 as usize].push(bi);
            }
            _ => {}
        }
    }
    preds
}

fn rpo(func: &Function) -> Vec<usize> {
    let mut seen = vec![false; func.blocks.len()];
    let mut post = Vec::new();
    dfs(func.entry.0 as usize, func, &mut seen, &mut post);
    post.reverse();
    post
}

fn dfs(b: usize, func: &Function, seen: &mut [bool], post: &mut Vec<usize>) {
    if seen[b] {
        return;
    }
    seen[b] = true;
    if let Some(term) = func.blocks[b].insts.last() {
        match term {
            Inst::Jump(t, _) => dfs(t.0 as usize, func, seen, post),
            Inst::Brif(_, t, _, f, _) => {
                dfs(t.0 as usize, func, seen, post);
                dfs(f.0 as usize, func, seen, post);
            }
            _ => {}
        }
    }
    post.push(b);
}

fn dominators(func: &Function, preds: &[Vec<usize>], order: &[usize]) -> Vec<Option<usize>> {
    let n = func.blocks.len();
    let mut rpo_index = vec![usize::MAX; n];
    for (i, &b) in order.iter().enumerate() {
        rpo_index[b] = i;
    }
    let entry = func.entry.0 as usize;
    let mut idom = vec![None; n];
    idom[entry] = Some(entry);
    let mut changed = true;
    while changed {
        changed = false;
        for &b in order {
            if b == entry {
                continue;
            }
            let mut new_idom = None;
            for &p in &preds[b] {
                if idom[p].is_none() {
                    continue;
                }
                new_idom = Some(match new_idom {
                    None => p,
                    Some(cur) => intersect(p, cur, &idom, &rpo_index),
                });
            }
            if let Some(new) = new_idom {
                if idom[b] != Some(new) {
                    idom[b] = Some(new);
                    changed = true;
                }
            }
        }
    }
    idom
}

fn intersect(mut a: usize, mut b: usize, idom: &[Option<usize>], rpo_index: &[usize]) -> usize {
    while a != b {
        while rpo_index[a] > rpo_index[b] {
            a = idom[a].expect("dominator");
        }
        while rpo_index[b] > rpo_index[a] {
            b = idom[b].expect("dominator");
        }
    }
    a
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
