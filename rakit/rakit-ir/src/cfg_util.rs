//! Shared CFG rewriting helpers: remove blocks or values and renumber every
//! remaining reference so the IR stays well formed.

use std::collections::HashSet;

use crate::ir::{Function, Inst, Value};

pub(crate) fn remove_blocks(func: &mut Function, remove: &[usize]) {
    let remove: HashSet<usize> = remove.iter().copied().collect();
    let mut map = vec![None; func.blocks.len()];
    let mut next = 0u32;
    for (old, slot) in map.iter_mut().enumerate() {
        if !remove.contains(&old) {
            *slot = Some(next);
            next += 1;
        }
    }
    let old_blocks = std::mem::take(&mut func.blocks);
    let mut blocks = Vec::with_capacity(next as usize);
    for (old, mut block) in old_blocks.into_iter().enumerate() {
        if remove.contains(&old) {
            continue;
        }
        for inst in &mut block.insts {
            match inst {
                Inst::Jump(t, _) => {
                    t.0 = map[t.0 as usize].expect("branch to removed block");
                }
                Inst::Brif(_, t, _, f, _) => {
                    t.0 = map[t.0 as usize].expect("branch to removed block");
                    f.0 = map[f.0 as usize].expect("branch to removed block");
                }
                _ => {}
            }
        }
        blocks.push(block);
    }
    func.blocks = blocks;
    func.entry.0 = map[func.entry.0 as usize].expect("entry block was removed");
}

pub(crate) fn remove_values(func: &mut Function, removed: &HashSet<Value>) {
    let mut map = vec![None; func.values.len()];
    let mut next = 0u32;
    for (old, slot) in map.iter_mut().enumerate() {
        if !removed.contains(&Value(old as u32)) {
            *slot = Some(Value(next));
            next += 1;
        }
    }
    let old_values = std::mem::take(&mut func.values);
    func.values = old_values
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(&Value(*i as u32)))
        .map(|(_, ty)| ty)
        .collect();
    for block in &mut func.blocks {
        for p in &mut block.params {
            *p = map[p.0 as usize].expect("block param removed");
        }
        for inst in &mut block.insts {
            remap_inst_values(inst, &map);
        }
    }
}

pub(crate) fn predecessors(func: &Function) -> Vec<Vec<usize>> {
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

pub(crate) fn rpo(func: &Function) -> Vec<usize> {
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

pub(crate) fn dominators(
    func: &Function,
    preds: &[Vec<usize>],
    order: &[usize],
) -> Vec<Option<usize>> {
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

fn intersect(a: usize, b: usize, idom: &[Option<usize>], rpo_index: &[usize]) -> usize {
    let mut a = a;
    let mut b = b;
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

pub(crate) fn dominates(a: usize, b: usize, idom: &[Option<usize>]) -> bool {
    let mut cur = b;
    loop {
        if cur == a {
            return true;
        }
        match idom[cur] {
            Some(d) if d != cur => cur = d,
            _ => return false,
        }
    }
}

fn remap_inst_values(inst: &mut Inst, map: &[Option<Value>]) {
    fn r(map: &[Option<Value>], v: &mut Value) {
        *v = map[v.0 as usize].expect("value removed but still used");
    }
    match inst {
        Inst::Nop(v, _)
        | Inst::Iconst(v, _, _)
        | Inst::F32const(v, _)
        | Inst::F64const(v, _)
        | Inst::Copy(v, _, _)
        | Inst::Bin(v, _, _, _, _)
        | Inst::Icmp(v, _, _, _, _)
        | Inst::Fcmp(v, _, _, _, _)
        | Inst::Select(v, _, _, _, _)
        | Inst::StackAddr(v, _, _)
        | Inst::Load(v, _, _, _) => r(map, v),
        _ => {}
    }
    match inst {
        Inst::Nop(..)
        | Inst::Iconst(..)
        | Inst::F32const(..)
        | Inst::F64const(..)
        | Inst::StackAddr(..) => {}
        Inst::Copy(_, _, x) => r(map, x),
        Inst::Bin(_, _, _, x, y) | Inst::Icmp(_, _, _, x, y) | Inst::Fcmp(_, _, _, x, y) => {
            r(map, x);
            r(map, y);
        }
        Inst::Select(_, _, c, x, y) => {
            r(map, c);
            r(map, x);
            r(map, y);
        }
        Inst::Load(_, _, a, _) => r(map, a),
        Inst::Store(_, a, _, v) => {
            r(map, a);
            r(map, v);
        }
        Inst::Jump(_, args) | Inst::Return(args) => {
            for a in args {
                r(map, a);
            }
        }
        Inst::Brif(c, _, ta, _, fa) => {
            r(map, c);
            for a in ta {
                r(map, a);
            }
            for a in fa {
                r(map, a);
            }
        }
    }
}
