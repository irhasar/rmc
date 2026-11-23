//! If-conversion of two-way diamonds on Rakit IR.
//!
//! A short `if cond { a } else { b }` that only computes a value lowers to a
//! diamond: a `brif` from the head to a *then* and an *else* block, both of
//! which `jump` to a common *join* that receives the chosen value as a
//! parameter. When the arms are small and side-effect-free, the diamond is
//! equivalent to a `select` in the head, which Rakit lowers to a `cmove`.
//!
//! The rewrite is deliberately conservative: both arms must be single-
//! predecessor, single-successor, side-effect-free blocks that end in a `jump`
//! to the same join, carry no block parameters, and define no value that is used
//! outside them.

use std::collections::{HashMap, HashSet};

use crate::cfg_util::{remove_blocks, remove_values};
use crate::ir::{BinOp, Block, BlockData, Function, Inst, Type, Value};

/// Cap on the number of instructions an arm may contain.
const MAX_ARM_INSTS: usize = 8;

pub fn run(func: &mut Function) -> bool {
    let mut changed = false;
    while convert_one(func) {
        changed = true;
    }
    changed
}

struct ArmPlan {
    insts: Vec<Inst>,
    values: Vec<Value>,
    defined: HashSet<Value>,
}

fn convert_one(func: &mut Function) -> bool {
    let (preds, succs) = cfg(func);
    let block_count = func.blocks.len();
    for head in 0..block_count {
        let Some(Inst::Brif(cond, then_b, then_args, else_b, else_args)) =
            func.blocks[head].insts.last().cloned()
        else {
            continue;
        };
        let (tb, eb) = (then_b.0 as usize, else_b.0 as usize);
        if tb >= block_count || eb >= block_count {
            continue;
        }
        if then_b == else_b || tb == head || eb == head {
            continue;
        }
        if !then_args.is_empty() || !else_args.is_empty() {
            continue;
        }
        if !func.blocks[tb].params.is_empty() || !func.blocks[eb].params.is_empty() {
            continue;
        }
        if preds[tb] != 1 || preds[eb] != 1 {
            continue;
        }
        let Some(then_end) = arm_end(&func.blocks[tb]) else {
            continue;
        };
        let Some(else_end) = arm_end(&func.blocks[eb]) else {
            continue;
        };
        let (join, then_values, else_values, value_tys, is_return) = match (&then_end, &else_end) {
            (ArmEnd::Jump(tj, tv), ArmEnd::Jump(ej, ev)) => {
                if tj != ej || tv.len() != ev.len() || succs[tb] != 1 || succs[eb] != 1 {
                    continue;
                }
                let jb = tj.0 as usize;
                if jb >= block_count || jb == head || jb == tb || jb == eb {
                    continue;
                }
                let params = &func.blocks[jb].params;
                if params.len() != tv.len() {
                    continue;
                }
                let tys: Vec<Type> = params.iter().map(|p| func.values[p.0 as usize]).collect();
                (Some(*tj), tv.clone(), ev.clone(), tys, false)
            }
            (ArmEnd::Return(tv), ArmEnd::Return(ev)) => {
                if tv.len() != ev.len()
                    || tv.len() != func.ret.len()
                    || succs[tb] != 0
                    || succs[eb] != 0
                {
                    continue;
                }
                (None, tv.clone(), ev.clone(), func.ret.clone(), true)
            }
            _ => continue,
        };
        let Some(then_plan) = analyze_arm(&func.blocks[tb], then_values) else {
            continue;
        };
        let Some(else_plan) = analyze_arm(&func.blocks[eb], else_values) else {
            continue;
        };
        if !values_dead_outside(func, tb, eb, &then_plan, &else_plan) {
            continue;
        }
        if uses_other_arm(&then_plan, &else_plan.defined)
            || uses_other_arm(&else_plan, &then_plan.defined)
        {
            continue;
        }

        let mut removed_values = HashSet::new();
        removed_values.extend(then_plan.defined.iter().copied());
        removed_values.extend(else_plan.defined.iter().copied());

        let mut then_map = HashMap::new();
        let Some(mut then_new) = clone_arm(func, &then_plan, &mut then_map) else {
            continue;
        };
        let mut else_map = HashMap::new();
        let Some(mut else_new) = clone_arm(func, &else_plan, &mut else_map) else {
            continue;
        };
        let then_vals: Vec<Value> = then_plan.values.iter().map(|v| remap(*v, &then_map)).collect();
        let else_vals: Vec<Value> = else_plan.values.iter().map(|v| remap(*v, &else_map)).collect();
        let mut selects = Vec::with_capacity(value_tys.len());
        let mut sels = Vec::with_capacity(value_tys.len());
        for i in 0..value_tys.len() {
            let sel = new_value(func, value_tys[i]);
            sels.push(sel);
            selects.push((sel, value_tys[i], then_vals[i], else_vals[i]));
        }

        let head_block = &mut func.blocks[head];
        head_block.insts.pop();
        head_block.insts.append(&mut then_new);
        head_block.insts.append(&mut else_new);
        for (sel, ty, tv, ev) in selects {
            head_block.insts.push(Inst::Select(sel, ty, cond, tv, ev));
        }
        if is_return {
            head_block.insts.push(Inst::Return(sels));
        } else {
            head_block.insts.push(Inst::Jump(join.expect("jump arm has a join"), sels));
        }
        remove_blocks(func, &[tb, eb]);
        remove_values(func, &removed_values);
        return true;
    }
    false
}

fn cfg(func: &Function) -> (Vec<usize>, Vec<usize>) {
    let n = func.blocks.len();
    let mut preds = vec![0usize; n];
    let mut succs = vec![0usize; n];
    for (bi, block) in func.blocks.iter().enumerate() {
        let Some(term) = block.insts.last() else {
            continue;
        };
        match term {
            Inst::Jump(t, _) => {
                preds[t.0 as usize] += 1;
                succs[bi] += 1;
            }
            Inst::Brif(_, t, _, f, _) => {
                preds[t.0 as usize] += 1;
                preds[f.0 as usize] += 1;
                succs[bi] += 2;
            }
            _ => {}
        }
    }
    (preds, succs)
}

enum ArmEnd {
    Jump(Block, Vec<Value>),
    Return(Vec<Value>),
}

fn arm_end(block: &BlockData) -> Option<ArmEnd> {
    match block.insts.last()? {
        Inst::Jump(t, args) => Some(ArmEnd::Jump(*t, args.clone())),
        Inst::Return(args) => Some(ArmEnd::Return(args.clone())),
        _ => None,
    }
}

fn analyze_arm(block: &BlockData, values: Vec<Value>) -> Option<ArmPlan> {
    let n = block.insts.len();
    if n == 0 {
        return None;
    }
    let mut insts = Vec::new();
    let mut defined = HashSet::new();
    for inst in &block.insts[..n - 1] {
        if inst.is_terminator() {
            return None;
        }
        if insts.len() >= MAX_ARM_INSTS {
            return None;
        }
        if !is_cheap(inst) {
            return None;
        }
        if let Some(r) = inst.result_value() {
            defined.insert(r);
        }
        insts.push(inst.clone());
    }
    Some(ArmPlan { insts, values, defined })
}

fn is_cheap(inst: &Inst) -> bool {
    match inst {
        Inst::Nop(..)
        | Inst::Iconst(..)
        | Inst::F32const(..)
        | Inst::F64const(..)
        | Inst::Copy(..)
        | Inst::Icmp(..)
        | Inst::Fcmp(..)
        | Inst::Select(..) => true,
        Inst::Bin(_, op, _, _, _) => {
            !matches!(op, BinOp::Sdiv | BinOp::Udiv | BinOp::Srem | BinOp::Urem | BinOp::Fdiv)
        }
        _ => false,
    }
}

fn values_dead_outside(
    func: &Function,
    then_block: usize,
    else_block: usize,
    then_plan: &ArmPlan,
    else_plan: &ArmPlan,
) -> bool {
    let mut defined = HashSet::new();
    defined.extend(then_plan.defined.iter().copied());
    defined.extend(else_plan.defined.iter().copied());
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == then_block || bi == else_block {
            continue;
        }
        for inst in &block.insts {
            if inst.operands().iter().any(|v| defined.contains(v)) {
                return false;
            }
        }
    }
    true
}

fn uses_other_arm(plan: &ArmPlan, other: &HashSet<Value>) -> bool {
    plan.insts.iter().any(|inst| inst.operands().iter().any(|v| other.contains(v)))
        || plan.values.iter().any(|v| other.contains(v))
}

fn clone_arm(
    func: &mut Function,
    plan: &ArmPlan,
    map: &mut HashMap<Value, Value>,
) -> Option<Vec<Inst>> {
    let mut out = Vec::with_capacity(plan.insts.len());
    for inst in &plan.insts {
        out.push(clone_inst(func, inst, map)?);
    }
    Some(out)
}

fn clone_inst(func: &mut Function, inst: &Inst, map: &mut HashMap<Value, Value>) -> Option<Inst> {
    let old = inst.result_value()?;
    let ty = *func.values.get(old.0 as usize)?;
    let new = new_value(func, ty);
    let out = match inst {
        Inst::Nop(_, ty) => Inst::Nop(new, *ty),
        Inst::Iconst(_, ty, v) => Inst::Iconst(new, *ty, *v),
        Inst::F32const(_, v) => Inst::F32const(new, *v),
        Inst::F64const(_, v) => Inst::F64const(new, *v),
        Inst::Copy(_, ty, x) => Inst::Copy(new, *ty, remap(*x, map)),
        Inst::Bin(_, op, ty, x, y) => Inst::Bin(new, *op, *ty, remap(*x, map), remap(*y, map)),
        Inst::Icmp(_, cc, ty, x, y) => Inst::Icmp(new, *cc, *ty, remap(*x, map), remap(*y, map)),
        Inst::Fcmp(_, cc, ty, x, y) => Inst::Fcmp(new, *cc, *ty, remap(*x, map), remap(*y, map)),
        Inst::Call(..) | Inst::CallIndirect { .. } => return None,
        Inst::Select(_, ty, c, x, y) => {
            Inst::Select(new, *ty, remap(*c, map), remap(*x, map), remap(*y, map))
        }
        Inst::StackAddr(..)
        | Inst::Load(..)
        | Inst::Store(..)
        | Inst::Jump(..)
        | Inst::Brif(..)
        | Inst::Return(..) => return None,
    };
    map.insert(old, new);
    Some(out)
}

fn remap(v: Value, map: &HashMap<Value, Value>) -> Value {
    *map.get(&v).unwrap_or(&v)
}

fn new_value(func: &mut Function, ty: Type) -> Value {
    let v = Value(func.values.len() as u32);
    func.values.push(ty);
    v
}
