//! Graph-colouring register allocation with spill slots.

use std::cmp::Reverse;
use std::collections::HashSet;

use crate::backend::mach::{ALLOCATABLE, MachFunction, Operand, PReg, VReg};
use crate::backend::{AllocStats, BackendError, RegisterAllocator};

pub struct LinearScan;

impl RegisterAllocator for LinearScan {
    fn allocate(&self, func: &mut MachFunction) -> Result<AllocStats, BackendError> {
        let nregs = func.vregs.len();
        let nblocks = func.blocks.len();

        let mut succ = vec![Vec::new(); nblocks];
        for b in 0..nblocks {
            succ[b] = func.successors(b);
        }

        let mut block_use = vec![HashSet::new(); nblocks];
        let mut block_def = vec![HashSet::new(); nblocks];
        for b in 0..nblocks {
            let mut defined = HashSet::new();
            for inst in &func.blocks[b].insts {
                for u in inst.uses() {
                    if !defined.contains(&u) {
                        block_use[b].insert(u);
                    }
                }
                for d in inst.defs() {
                    defined.insert(d);
                    block_def[b].insert(d);
                }
            }
        }

        let mut live_in = vec![HashSet::new(); nblocks];
        let mut live_out = vec![HashSet::new(); nblocks];
        loop {
            let mut changed = false;
            for b in (0..nblocks).rev() {
                let mut out = HashSet::new();
                for s in &succ[b] {
                    out.extend(live_in[*s].iter().copied());
                }
                let mut in_ = block_use[b].clone();
                for v in out.iter() {
                    if !block_def[b].contains(v) {
                        in_.insert(*v);
                    }
                }
                if out != live_out[b] || in_ != live_in[b] {
                    live_out[b] = out;
                    live_in[b] = in_;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let mut graph = vec![HashSet::new(); nregs];
        for b in 0..nblocks {
            let entry_live: Vec<_> = live_in[b].iter().copied().collect();
            for i in 0..entry_live.len() {
                for j in (i + 1)..entry_live.len() {
                    graph[entry_live[i].0 as usize].insert(entry_live[j]);
                    graph[entry_live[j].0 as usize].insert(entry_live[i]);
                }
            }
            let mut live = live_out[b].clone();
            for inst in func.blocks[b].insts.iter().rev() {
                let defs = inst.defs();
                for d in &defs {
                    for l in &live {
                        if *l != *d {
                            graph[d.0 as usize].insert(*l);
                            graph[l.0 as usize].insert(*d);
                        }
                    }
                }
                let uses = inst.uses();
                for i in 0..uses.len() {
                    for j in (i + 1)..uses.len() {
                        if uses[i] != uses[j] {
                            graph[uses[i].0 as usize].insert(uses[j]);
                            graph[uses[j].0 as usize].insert(uses[i]);
                        }
                    }
                }
                for d in defs {
                    live.remove(&d);
                }
                for u in uses {
                    live.insert(u);
                }
            }
        }

        let mut order: Vec<VReg> = (0..nregs as u32).map(VReg).collect();
        order.sort_by_key(|v| Reverse(graph[v.0 as usize].len()));

        let mut colors: Vec<Option<PReg>> = vec![None; nregs];
        let mut spills: Vec<Option<u32>> = vec![None; nregs];
        let mut spill_count = 0u32;

        for v in order {
            let mut used = HashSet::new();
            for nbr in &graph[v.0 as usize] {
                if let Some(color) = colors[nbr.0 as usize] {
                    used.insert(color);
                }
            }
            if let Some(color) = ALLOCATABLE.iter().copied().find(|c| !used.contains(c)) {
                colors[v.0 as usize] = Some(color);
            } else {
                spills[v.0 as usize] = Some(spill_count);
                spill_count += 1;
            }
        }

        for block in &mut func.blocks {
            for inst in &mut block.insts {
                inst.map_vregs(&mut |v| location(&colors, &spills, v));
            }
        }
        func.param_locs = func.params.iter().map(|v| location(&colors, &spills, *v)).collect();
        func.spill_slots = spill_count;

        let registers = colors.iter().filter(|c| c.is_some()).count();
        Ok(AllocStats { vregs: nregs, registers, spills: spill_count as usize })
    }
}

fn location(colors: &[Option<PReg>], spills: &[Option<u32>], v: VReg) -> Operand {
    if let Some(reg) = colors[v.0 as usize] {
        Operand::PReg(reg)
    } else if let Some(slot) = spills[v.0 as usize] {
        Operand::Spill(slot)
    } else {
        Operand::PReg(ALLOCATABLE[0])
    }
}
