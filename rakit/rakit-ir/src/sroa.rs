//! Promote scalar stack slots to SSA values.
//!
//! Only slots whose address never escapes and whose accesses are all constant
//! scalar loads/stores are promoted. Phi nodes are inserted as block parameters
//! at dominance-frontier joins, then a dominator-tree rename replaces loads
//! with the current SSA value and drops the stores. Slots left without any
//! `StackAddr` are removed and the remaining slots are renumbered.

use std::collections::{HashMap, HashSet};

use crate::cfg_util::{dominators, predecessors, rpo};
use crate::ir::{Function, Inst, Type, Value};
use crate::mem_forward::{Key, definitions, escaping_slots, key_of, size_of};

type Loc = (i64, Type);

/// A dynamic address into a one-element slot must be the element itself for
/// defined programs: any other index is out of bounds. Such accesses are
/// therefore promoted to the same SSA location as a constant offset zero.
fn access_loc(_func: &Function, key: Key, target: u32, ty: Type) -> Option<Loc> {
    match key {
        Key::Const { slot, offset } if slot == target => Some((offset, ty)),
        // Dynamic indexing is left to `mem_forward`; a slot can be written
        // through one dynamic address and read through another, so promoting
        // it to a single SSA location is not sound in general.
        Key::Dyn { .. } => None,
        _ => None,
    }
}

pub fn run(func: &mut Function) -> bool {
    let original = func.clone();
    let mut changed = false;
    let slots = func.stack_slots.len();
    for slot in 0..slots {
        let mut attempt = func.clone();
        if promote_slot(&mut attempt, slot as u32) {
            *func = attempt;
            changed = true;
        }
    }
    if changed {
        crate::optimize(func);
    }
    if remove_unused_slots(func) {
        changed = true;
    }
    if changed {
        crate::optimize(func);
    }
    if changed && crate::verify(func).is_err() {
        *func = original;
        return false;
    }
    changed
}

fn new_value(func: &mut Function, ty: Type) -> Value {
    let value = Value(func.values.len() as u32);
    func.values.push(ty);
    value
}

fn promote_slot(func: &mut Function, target: u32) -> bool {
    if func.stack_slots.get(target as usize).is_none() {
        return false;
    }
    // Keep promotion to a single straight-line block.  The std slice-pattern
    // suite exposed a miscompile when an entry block was also a loop header,
    // so a function with any control flow is left alone until the cross-block
    // must-store proof lands.
    if func.blocks.len() != 1
        || !matches!(func.blocks[0].insts.last(), Some(Inst::Return(_)))
    {
        return false;
    }
    // A call can observe memory through an alias that the address-use scan
    // cannot prove private.  Keep promotion to leaf code until alias analysis
    // exists.
    if func.blocks.iter().any(|b| {
        b.insts.iter().any(|i| {
            matches!(
                i,
                Inst::Call(..)
                    | Inst::CallIndirect { .. }
                    | Inst::TryCall { .. }
                    | Inst::TryCallIndirect { .. }
            )
        })
    }) {
        return false;
    }
    let defs = definitions(func);
    let escaping = escaping_slots(func, &defs);
    if escaping[target as usize] {
        return false;
    }

    let order = rpo(func);
    let reachable: HashSet<usize> = order.iter().copied().collect();
    let preds = predecessors(func);
    let idom = dominators(func, &preds, &order);
    let mut children = vec![Vec::new(); func.blocks.len()];
    for (b, parent) in idom.iter().enumerate() {
        if let Some(p) = parent {
            if *p != b {
                children[*p].push(b);
            }
        }
    }
    let df = dominance_frontier(func.blocks.len(), &idom, &preds);

    let mut keys: HashSet<Loc> = HashSet::new();
    let mut def_blocks: HashMap<Loc, HashSet<usize>> = HashMap::new();
    let mut access_blocks: HashSet<usize> = HashSet::new();

    for (bi, block) in func.blocks.iter().enumerate() {
        for inst in &block.insts {
            match inst {
                Inst::Load(_, ty, addr, offset, _) => {
                    let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                        continue;
                    };
                    if key.slot() == target && !matches!(key, Key::Const { .. }) {
                        return false;
                    }
                    if let Some(loc) = access_loc(func, key, target, *ty) {
                        keys.insert(loc);
                        access_blocks.insert(bi);
                    }
                }
                Inst::Store(ty, addr, offset, value, _) => {
                    if func.ty(*value) != *ty {
                        continue;
                    }
                    let Some(key) = key_of(&defs, func, *addr, *offset, *ty) else {
                        continue;
                    };
                    if key.slot() == target && !matches!(key, Key::Const { .. }) {
                        return false;
                    }
                    if let Some(loc) = access_loc(func, key, target, *ty) {
                        keys.insert(loc);
                        def_blocks.entry(loc).or_default().insert(bi);
                        access_blocks.insert(bi);
                    }
                }
                _ => {}
            }
        }
    }

    if keys.is_empty() || access_blocks.iter().any(|b| !reachable.contains(b)) {
        return false;
    }
    // Keep the first promotion tier to a whole scalar slot.  Sub-slot and
    // aggregate promotion needs a field-sensitive proof that std slice
    // patterns exposed as unsound.
    let slot_size = func.stack_slots[target as usize].size;
    if keys.iter().any(|&(offset, ty)| offset != 0 || size_of(ty) != slot_size) {
        return false;
    }
    // Cross-block promotion needs a complete must-store proof at every phi
    // edge.  Until that is in place, keep SROA to straight-line blocks; the
    // std slice-pattern suite exposed a cross-block case this pass got wrong.
    if access_blocks.len() != 1 || !access_blocks.contains(&(func.entry.0 as usize)) {
        return false;
    }

    let ranges: Vec<(i64, i64)> =
        keys.iter().map(|&(offset, ty)| (offset, offset + size_of(ty) as i64)).collect();
    for i in 0..ranges.len() {
        for j in i + 1..ranges.len() {
            let (a, b) = ranges[i];
            let (c, d) = ranges[j];
            if a < d && c < b {
                return false;
            }
        }
    }

    let mut phi: HashMap<(usize, Loc), Value> = HashMap::new();
    let mut phi_order: Vec<Vec<Loc>> = vec![Vec::new(); func.blocks.len()];

    for &loc in &keys {
        let Some(blocks) = def_blocks.get(&loc) else {
            continue;
        };
        let mut work: Vec<usize> = blocks.iter().copied().collect();
        let mut queued: HashSet<usize> = work.iter().copied().collect();
        while let Some(d) = work.pop() {
            queued.remove(&d);
            for &y in &df[d] {
                if y == func.entry.0 as usize {
                    return false;
                }
                if preds[y].iter().any(|p| !reachable.contains(p)) {
                    return false;
                }
                if phi.contains_key(&(y, loc)) {
                    continue;
                }
                let value = new_value(func, loc.1);
                func.blocks[y].params.push(value);
                phi.insert((y, loc), value);
                phi_order[y].push(loc);
                if !blocks.contains(&y) && queued.insert(y) {
                    work.push(y);
                }
            }
        }
    }

    let mut succ = vec![Vec::new(); func.blocks.len()];
    for (b, ps) in preds.iter().enumerate() {
        for &p in ps {
            succ[p].push(b);
        }
    }

    let mut value_block = HashMap::new();
    for (b, block) in func.blocks.iter().enumerate() {
        for p in &block.params {
            value_block.insert(*p, b);
        }
        for inst in &block.insts {
            if let Some(r) = inst.result_value() {
                value_block.insert(r, b);
            }
        }
    }
    let mut renamer = Renamer {
        func,
        defs,
        target,
        phi,
        phi_order,
        children,
        succ,
        idom,
        value_block,
        current: HashMap::new(),
    };
    if !renamer.block(renamer.func.entry.0 as usize) {
        return false;
    }
    true
}

fn dominance_frontier(n: usize, idom: &[Option<usize>], preds: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut df = vec![Vec::new(); n];
    for b in 0..n {
        if preds[b].len() < 2 {
            continue;
        }
        let stop = idom[b];
        for &p in &preds[b] {
            let mut runner = p;
            while Some(runner) != stop {
                if !df[runner].contains(&b) {
                    df[runner].push(b);
                }
                match idom[runner] {
                    Some(next) => runner = next,
                    None => break,
                }
            }
        }
    }
    df
}

fn append_args(func: &mut Function, b: usize, succ: usize, args: &[Value]) -> bool {
    let Some(term) = func.blocks[b].insts.last_mut() else {
        return false;
    };
    match term {
        Inst::Jump(target, target_args) if target.0 as usize == succ => {
            target_args.extend_from_slice(args);
            true
        }
        Inst::Brif(_, then_block, then_args, else_block, else_args) => {
            let mut ok = false;
            if then_block.0 as usize == succ {
                then_args.extend_from_slice(args);
                ok = true;
            }
            if else_block.0 as usize == succ {
                else_args.extend_from_slice(args);
                ok = true;
            }
            ok
        }
        _ => false,
    }
}

struct Renamer<'a> {
    func: &'a mut Function,
    defs: HashMap<Value, Inst>,
    target: u32,
    phi: HashMap<(usize, Loc), Value>,
    phi_order: Vec<Vec<Loc>>,
    children: Vec<Vec<usize>>,
    succ: Vec<Vec<usize>>,
    idom: Vec<Option<usize>>,
    value_block: HashMap<Value, usize>,
    current: HashMap<Loc, Value>,
}

impl Renamer<'_> {
    fn available(&self, value: Value, b: usize) -> bool {
        match self.value_block.get(&value) {
            Some(&def) => def == b || crate::cfg_util::dominates(def, b, &self.idom),
            None => false,
        }
    }

    fn block(&mut self, b: usize) -> bool {
        let order = self.phi_order[b].clone();
        let mut saved = Vec::new();
        for loc in order {
            let old = self.current.insert(loc, self.phi[&(b, loc)]);
            saved.push((loc, old));
        }

        let old_insts = std::mem::take(&mut self.func.blocks[b].insts);
        let mut insts = Vec::with_capacity(old_insts.len());
        for inst in old_insts {
            match &inst {
                Inst::Load(r, ty, addr, offset, _) => {
                    if let Some(key) = key_of(&self.defs, self.func, *addr, *offset, *ty) {
                        if let Some(loc) = access_loc(self.func, key, self.target, *ty) {
                            let Some(value) = self.current.get(&loc).copied() else {
                                return false;
                            };
                            if !self.available(value, b) {
                                return false;
                            }
                            insts.push(Inst::Copy(*r, *ty, value));
                            continue;
                        }
                    }
                    insts.push(inst);
                }
                Inst::Store(ty, addr, offset, value, _) => {
                    if self.func.ty(*value) != *ty {
                        insts.push(inst);
                        continue;
                    }
                    if let Some(key) = key_of(&self.defs, self.func, *addr, *offset, *ty) {
                        if let Some(loc) = access_loc(self.func, key, self.target, *ty) {
                            self.current.insert(loc, *value);
                            continue;
                        }
                    }
                    insts.push(inst);
                }
                _ => insts.push(inst),
            }
        }
        self.func.blocks[b].insts = insts;

        let mut seen = HashSet::new();
        for &next in &self.succ[b] {
            if !seen.insert(next) {
                continue;
            }
            let args: Option<Vec<Value>> =
                self.phi_order[next].iter().map(|loc| self.current.get(loc).copied()).collect();
            let Some(args) = args else {
                return false;
            };
            if !args.is_empty() && !append_args(self.func, b, next, &args) {
                return false;
            }
        }

        let children = self.children[b].clone();
        for child in children {
            if !self.block(child) {
                return false;
            }
        }

        for (loc, old) in saved {
            match old {
                Some(value) => {
                    self.current.insert(loc, value);
                }
                None => {
                    self.current.remove(&loc);
                }
            }
        }
        true
    }
}

fn remove_unused_slots(func: &mut Function) -> bool {
    let mut used = vec![false; func.stack_slots.len()];
    for block in &func.blocks {
        for inst in &block.insts {
            if let Inst::StackAddr(_, slot, _) = inst {
                if let Some(slot) = used.get_mut(slot.0 as usize) {
                    *slot = true;
                }
            }
        }
    }
    if used.iter().all(|&u| u) {
        return false;
    }

    let mut remap = vec![None; used.len()];
    let mut next = 0u32;
    for (old, &is_used) in used.iter().enumerate() {
        if is_used {
            remap[old] = Some(next);
            next += 1;
        }
    }
    let slots: Vec<_> = func
        .stack_slots
        .iter()
        .enumerate()
        .filter(|(i, _)| used[*i])
        .map(|(_, data)| data.clone())
        .collect();
    func.stack_slots = slots;
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            if let Inst::StackAddr(_, slot, _) = inst {
                if let Some(new) = remap[slot.0 as usize] {
                    slot.0 = new;
                }
            }
        }
    }
    true
}
