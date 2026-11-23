//! CFG simplification: drop blocks that are unreachable from the entry and
//! collapse blocks that only forward to another block.

use std::collections::HashSet;

use crate::cfg_util::{predecessors, remove_blocks, remove_values};
use crate::ir::{Function, Inst, Value};

pub fn run(func: &mut Function) -> bool {
    let original = func.clone();
    let mut changed = false;
    loop {
        let unreachable = unreachable_blocks(func);
        if !unreachable.is_empty() {
            remove_blocks_and_values(func, &unreachable);
            changed = true;
            continue;
        }
        if let Some(b) = find_forwarder(func) {
            redirect_to_target(func, b);
            remove_blocks_and_values(func, &[b]);
            changed = true;
            continue;
        }
        let Some((p, b)) = find_merge(func) else {
            break;
        };
        merge_blocks(func, p, b);
        changed = true;
    }
    if changed && crate::verify(func).is_err() {
        *func = original;
        return false;
    }
    changed
}

fn unreachable_blocks(func: &Function) -> Vec<usize> {
    let mut seen = vec![false; func.blocks.len()];
    let mut stack = vec![func.entry.0 as usize];
    seen[func.entry.0 as usize] = true;
    while let Some(b) = stack.pop() {
        let Some(term) = func.blocks[b].insts.last() else {
            continue;
        };
        match term {
            Inst::Jump(t, _) => push(t.0 as usize, &mut seen, &mut stack),
            Inst::Brif(_, t, _, f, _) => {
                push(t.0 as usize, &mut seen, &mut stack);
                push(f.0 as usize, &mut seen, &mut stack);
            }
            Inst::TryCall { normal, exception, .. }
            | Inst::TryCallIndirect { normal, exception, .. } => {
                push(normal.block.0 as usize, &mut seen, &mut stack);
                for item in exception {
                    match item {
                        crate::ir::ExceptionItem::Tag(_, call)
                        | crate::ir::ExceptionItem::Default(call) => {
                            push(call.block.0 as usize, &mut seen, &mut stack)
                        }
                        crate::ir::ExceptionItem::Context(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
    (0..func.blocks.len()).filter(|i| !seen[*i]).collect()
}

fn push(b: usize, seen: &mut [bool], stack: &mut Vec<usize>) {
    if !seen[b] {
        seen[b] = true;
        stack.push(b);
    }
}

fn find_forwarder(func: &Function) -> Option<usize> {
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == func.entry.0 as usize || !block.params.is_empty() {
            continue;
        }
        let Some(Inst::Jump(target, args)) = block.insts.last() else {
            continue;
        };
        if !args.is_empty()
            || target.0 as usize == bi
            || !func.blocks[target.0 as usize].params.is_empty()
        {
            continue;
        }
        if !block.insts[..block.insts.len() - 1].iter().all(|i| matches!(i, Inst::Nop(..))) {
            continue;
        }
        let mut defined = HashSet::new();
        for inst in &block.insts[..block.insts.len() - 1] {
            if let Some(v) = inst.result_value() {
                defined.insert(v);
            }
        }
        if !defined.is_empty() && !values_dead_outside(func, bi, &defined) {
            continue;
        }
        return Some(bi);
    }
    None
}

fn values_dead_outside(func: &Function, block: usize, defined: &HashSet<Value>) -> bool {
    for (bi, b) in func.blocks.iter().enumerate() {
        if bi == block {
            continue;
        }
        for inst in &b.insts {
            if inst.operands().iter().any(|v| defined.contains(v)) {
                return false;
            }
        }
    }
    true
}

fn find_merge(func: &Function) -> Option<(usize, usize)> {
    let preds = predecessors(func);
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == func.entry.0 as usize || !block.params.is_empty() {
            continue;
        }
        if preds[bi].len() != 1 {
            continue;
        }
        let p = preds[bi][0];
        if p == bi {
            continue;
        }
        let Some(Inst::Jump(t, args)) = func.blocks[p].insts.last() else {
            continue;
        };
        if t.0 as usize != bi || !args.is_empty() {
            continue;
        }
        return Some((p, bi));
    }
    None
}

fn merge_blocks(func: &mut Function, p: usize, b: usize) {
    let mut insts = std::mem::take(&mut func.blocks[b].insts);
    let p_insts = &mut func.blocks[p].insts;
    p_insts.pop();
    p_insts.append(&mut insts);
    remove_blocks(func, &[b]);
}

fn redirect_to_target(func: &mut Function, b: usize) {
    let target = {
        let Some(Inst::Jump(target, _)) = func.blocks[b].insts.last() else {
            unreachable!("forwarder must end in a jump");
        };
        *target
    };
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            match inst {
                Inst::Jump(t, _) if t.0 as usize == b => *t = target,
                Inst::Brif(_, t, _, f, _) => {
                    if t.0 as usize == b {
                        *t = target;
                    }
                    if f.0 as usize == b {
                        *f = target;
                    }
                }
                _ => {}
            }
        }
    }
}

fn remove_blocks_and_values(func: &mut Function, remove: &[usize]) {
    let mut removed = HashSet::new();
    for &bi in remove {
        let block = &func.blocks[bi];
        removed.extend(block.params.iter().copied());
        for inst in &block.insts {
            if let Some(v) = inst.result_value() {
                removed.insert(v);
            }
        }
    }
    remove_blocks(func, remove);
    if !removed.is_empty() {
        remove_values(func, &removed);
    }
}
