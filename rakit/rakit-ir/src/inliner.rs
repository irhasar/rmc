//! RIR function inliner.
//!
//! The codegen IR inliner works on codegen IR bodies collected per codegen unit.  This is
//! the RIR equivalent for the cases the bridge can represent: a direct call to
//! a small, single-block leaf is replaced by a copy of the callee body.

use crate::ir::{Function, Inst, Value};
use crate::unroll::{map_value, remap_inst};

pub const MAX_INLINE_INSTS: usize = 64;

pub fn run(func: &mut Function, bodies: &[Function]) -> bool {
    let mut changed = false;
    for bi in 0..func.blocks.len() {
        let old = std::mem::take(&mut func.blocks[bi].insts);
        let mut insts = Vec::with_capacity(old.len());
        for inst in old {
            let Some((results, target, args)) = call_parts(&inst) else {
                insts.push(inst);
                continue;
            };
            let Some(callee) = find_body(func, bodies, target, &args, &results) else {
                insts.push(inst);
                continue;
            };
            let mut map = std::collections::HashMap::new();
            for (p, a) in callee.block(callee.entry).params.iter().zip(&args) {
                map.insert(*p, *a);
            }
            let body_insts = &callee.block(callee.entry).insts;
            let Some(Inst::Return(rets)) = body_insts.last() else {
                insts.push(inst);
                continue;
            };
            let mut cloned = Vec::new();
            for body_inst in &body_insts[..body_insts.len() - 1] {
                if let Some(i) = remap_inst(func, &callee.values, body_inst, &mut map) {
                    cloned.push(i);
                } else {
                    cloned.clear();
                    break;
                }
            }
            if cloned.len() + 1 != body_insts.len() {
                insts.push(inst);
                continue;
            }
            for (old_r, new_v) in results.iter().zip(rets) {
                let ty = func.ty(*old_r);
                cloned.push(Inst::Copy(*old_r, ty, map_value(&map, *new_v)));
            }
            insts.extend(cloned);
            changed = true;
        }
        func.blocks[bi].insts = insts;
    }
    changed
}

fn call_parts(inst: &Inst) -> Option<(Vec<Value>, u32, Vec<Value>)> {
    match inst {
        Inst::Call(results, target, args) => Some((results.clone(), *target, args.clone())),
        _ => None,
    }
}

fn find_body<'a>(
    func: &Function,
    bodies: &'a [Function],
    target: u32,
    args: &[Value],
    results: &[Value],
) -> Option<&'a Function> {
    let ext = func.externals.get(target as usize)?;
    let body = bodies.iter().find(|b| b.self_target.as_ref() == Some(&ext.target))?;
    if body.blocks.len() != 1
        || !body.block(body.entry).params.len().eq(&args.len())
        || body.ret.len() != results.len()
        || !body.stack_slots.is_empty()
        || !body.dynamic_stack_slots.is_empty()
        || !body.globals.is_empty()
        || !body.externals.is_empty()
        || body.block(body.entry).insts.len() > MAX_INLINE_INSTS
    {
        return None;
    }
    for inst in &body.block(body.entry).insts {
        if matches!(
            inst,
            Inst::Call(..)
                | Inst::CallIndirect { .. }
                | Inst::TryCall { .. }
                | Inst::TryCallIndirect { .. }
                | Inst::Jump(..)
                | Inst::Brif(..)
        ) {
            return None;
        }
    }
    for (ty, arg) in body.ret.iter().zip(results) {
        if *ty != func.ty(*arg) {
            return None;
        }
    }
    Some(body)
}
