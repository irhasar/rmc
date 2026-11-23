//! Loop-invariant code motion: move pure instructions whose operands are
//! defined outside the loop into the loop preheader.

use std::collections::{BTreeMap, HashSet};

use crate::cfg_util::{dominates, dominators, predecessors, rpo};
use crate::ir::{BinOp, Function, Inst};

struct Loop {
    body: HashSet<usize>,
    preheader: usize,
}

pub fn run(func: &mut Function) -> bool {
    let mut changed = false;
    loop {
        let preds = predecessors(func);
        let order = rpo(func);
        let idom = dominators(func, &preds, &order);
        let loops = find_loops(func, &preds, &idom);
        let def = def_blocks(func);
        let mut moved_any = false;

        for lp in &loops {
            let pre = lp.preheader;
            let mut by_block: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
            for &b in &lp.body {
                let block = &func.blocks[b];
                for (i, inst) in block.insts.iter().enumerate() {
                    if i + 1 == block.insts.len() || !is_hoistable(inst) {
                        continue;
                    }
                    if !inst.operands().iter().all(|v| {
                        let Some(d) = def[v.0 as usize] else {
                            return false;
                        };
                        !lp.body.contains(&d) && dominates(d, pre, &idom)
                    }) {
                        continue;
                    }
                    by_block.entry(b).or_default().push(i);
                }
            }
            if by_block.is_empty() {
                continue;
            }

            let mut hoisted = Vec::new();
            for (b, mut idxs) in by_block {
                idxs.sort_unstable();
                let mut moved = Vec::new();
                for &i in idxs.iter().rev() {
                    moved.push(func.blocks[b].insts.remove(i));
                }
                moved.reverse();
                hoisted.extend(moved);
            }
            let pre_insts = &mut func.blocks[pre].insts;
            let term = pre_insts.pop().expect("preheader terminator");
            pre_insts.extend(hoisted);
            pre_insts.push(term);
            moved_any = true;
        }

        if !moved_any {
            break;
        }
        changed = true;
    }
    changed
}

fn successors(func: &Function, b: usize) -> Vec<usize> {
    match func.blocks[b].insts.last() {
        Some(Inst::Jump(t, _)) => vec![t.0 as usize],
        Some(Inst::Brif(_, t, _, f, _)) => vec![t.0 as usize, f.0 as usize],
        _ => Vec::new(),
    }
}

fn find_loops(func: &Function, preds: &[Vec<usize>], idom: &[Option<usize>]) -> Vec<Loop> {
    let mut headers: Vec<usize> = Vec::new();
    for b in 0..func.blocks.len() {
        for s in successors(func, b) {
            if dominates(s, b, idom) && !headers.contains(&s) {
                headers.push(s);
            }
        }
    }

    let mut loops = Vec::new();
    for &h in &headers {
        let mut body = HashSet::new();
        body.insert(h);
        for b in 0..func.blocks.len() {
            if !successors(func, b).contains(&h) || !dominates(h, b, idom) {
                continue;
            }
            let mut stack = vec![b];
            while let Some(x) = stack.pop() {
                if x == h || !body.insert(x) {
                    continue;
                }
                for &p in &preds[x] {
                    if p != h && dominates(h, p, idom) {
                        stack.push(p);
                    }
                }
            }
        }

        let outside: Vec<usize> = preds[h]
            .iter()
            .copied()
            .filter(|p| !body.contains(p))
            .collect();
        if outside.len() != 1 {
            continue;
        }
        let pre = outside[0];
        let Some(Inst::Jump(t, _)) = func.blocks[pre].insts.last() else {
            continue;
        };
        if t.0 as usize != h {
            continue;
        }
        if !body.iter().all(|&b| dominates(pre, b, idom)) {
            continue;
        }
        loops.push(Loop { body, preheader: pre });
    }
    loops
}

fn def_blocks(func: &Function) -> Vec<Option<usize>> {
    let mut def = vec![None; func.values.len()];
    for (bi, block) in func.blocks.iter().enumerate() {
        for &p in &block.params {
            def[p.0 as usize] = Some(bi);
        }
        for inst in &block.insts {
            if let Some(v) = inst.result_value() {
                def[v.0 as usize] = Some(bi);
            }
        }
    }
    def
}

fn is_hoistable(inst: &Inst) -> bool {
    match inst {
        Inst::Iconst(..) | Inst::F32const(..) | Inst::F64const(..) | Inst::Copy(..) => true,
        Inst::Bin(_, op, _, _, _) => !matches!(
            op,
            BinOp::Sdiv | BinOp::Udiv | BinOp::Srem | BinOp::Urem | BinOp::Fdiv
        ),
        Inst::Icmp(..) | Inst::Fcmp(..) | Inst::Select(..) => true,
        _ => false,
    }
}
