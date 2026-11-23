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
        | Inst::Select(v, _, _, _, _) => r(map, v),
        _ => {}
    }
    match inst {
        Inst::Nop(..) | Inst::Iconst(..) | Inst::F32const(..) | Inst::F64const(..) => {}
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
