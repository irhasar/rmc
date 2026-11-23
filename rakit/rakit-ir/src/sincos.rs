//! RIR port of the `sin(x)`/`cos(x)` fusion pass.
//!
//! LLVM (and the CLIF pass in `rakit-opt`) replaces a pair of libcalls with a
//! single `sincos`/`sincosf` call.  This RIR version handles the same-block
//! case: both calls must use the identical argument and the sine call must
//! precede the cosine call.

use std::collections::HashSet;

use crate::cfg_util::remove_values;
use crate::ir::{External, ExternalTarget, Function, Inst, Type, Value};

pub fn run(func: &mut Function) -> bool {
    let mut changed = false;
    let mut removed = HashSet::new();
    for bi in 0..func.blocks.len() {
        let mut i = 0;
        while i < func.blocks[bi].insts.len() {
            let Some((sin_name, sin_ty, sin_arg, sin_result)) = call_info(func, bi, i) else {
                i += 1;
                continue;
            };
            let cos_name = match sin_name {
                "sinf" => "cosf",
                "sin" => "cos",
                _ => {
                    i += 1;
                    continue;
                }
            };
            let mut found = None;
            for j in i + 1..func.blocks[bi].insts.len() {
                if let Some((name, ty, arg, result)) = call_info(func, bi, j) {
                    if name == cos_name && ty == sin_ty && arg == sin_arg {
                        found = Some((j, result));
                        break;
                    }
                }
            }
            let Some((j, cos_result)) = found else {
                i += 1;
                continue;
            };

            let target = if sin_ty == Type::F32 { "sincosf" } else { "sincos" };
            let ext = ensure_external(func, target, sin_ty);
            let sin_new = new_value(func, sin_ty);
            let cos_new = new_value(func, sin_ty);
            replace_uses(func, sin_result, sin_new);
            replace_uses(func, cos_result, cos_new);
            removed.insert(sin_result);
            removed.insert(cos_result);
            func.blocks[bi].insts.insert(i, Inst::Call(vec![sin_new, cos_new], ext, vec![sin_arg]));
            func.blocks[bi].insts.remove(i + 1);
            func.blocks[bi].insts.remove(j);
            changed = true;
            i += 1;
        }
    }
    if !removed.is_empty() {
        remove_values(func, &removed);
    }
    changed
}

fn call_info(func: &Function, bi: usize, ii: usize) -> Option<(&str, Type, Value, Value)> {
    let Inst::Call(results, target, args) = &func.blocks[bi].insts[ii] else {
        return None;
    };
    if results.len() != 1 || args.len() != 1 {
        return None;
    }
    let ext = func.externals.get(*target as usize)?;
    let ExternalTarget::LibCall(name) = &ext.target else {
        return None;
    };
    Some((name.as_str(), func.ty(results[0]), args[0], results[0]))
}

fn ensure_external(func: &mut Function, name: &str, ty: Type) -> u32 {
    if let Some(i) = func.externals.iter().position(|e| {
        e.target == ExternalTarget::LibCall(name.to_string()) && e.params == [ty] && e.rets == [ty, ty]
    }) {
        return i as u32;
    }
    func.externals.push(External {
        target: ExternalTarget::LibCall(name.to_string()),
        params: vec![ty],
        rets: vec![ty, ty],
    });
    (func.externals.len() - 1) as u32
}

fn new_value(func: &mut Function, ty: Type) -> Value {
    let v = Value(func.values.len() as u32);
    func.values.push(ty);
    v
}

fn replace_uses(func: &mut Function, old: Value, new: Value) {
    for block in &mut func.blocks {
        for inst in &mut block.insts {
            inst.replace_value(old, new);
        }
    }
}
