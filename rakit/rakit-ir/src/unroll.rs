//! RIR loop unrolling for counted loops.
//!
//! This is the RIR counterpart of the codegen IR unroller: it duplicates a
//! single-block loop body a fixed number of times.  It only fires when the
//! initial value, bound and step are compile-time constants and the trip count
//! is an exact multiple of the factor, so no remainder loop is needed.

use std::collections::{HashMap, HashSet};

use crate::cfg_util::remove_values;
use crate::ir::{BinOp, Block, BlockData, Function, Inst, IntCc, Type, Value};

pub const DEFAULT_FACTOR: usize = 2;

pub fn run(func: &mut Function) -> bool {
    let factor = std::env::var("RAKIT_RIR_UNROLL_FACTOR")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|f: &usize| *f >= 2)
        .unwrap_or(DEFAULT_FACTOR);
    let Some(plan) = match_loop(func, factor) else {
        return false;
    };
    rewrite(func, plan);
    true
}

struct Plan {
    header: usize,
    body: usize,
    body_args: Vec<Value>,
    back_args: Vec<Value>,
    factor: usize,
}

fn match_loop(func: &Function, factor: usize) -> Option<Plan> {
    for (header, block) in func.blocks.iter().enumerate() {
        let Some(Inst::Brif(cond, body_b, body_args, exit_b, _)) = block.insts.last() else {
            continue;
        };
        if *exit_b == Block(header as u32) || body_b.0 as usize == header {
            continue;
        }
        let body = body_b.0 as usize;
        let Some(body_block) = func.blocks.get(body) else { continue };
        let Some(Inst::Jump(back_b, back_args)) = body_block.insts.last() else { continue };
        if back_b.0 as usize != header {
            continue;
        }
        if block.params.len() != 2 || !body_block.params.is_empty() {
            continue;
        }
        let iv = block.params[0];
        let Some(bound) = block.insts.iter().find_map(|i| match i {
            Inst::Icmp(c, IntCc::Ult, Type::I64, x, y) if c == cond && *x == iv => {
                const_i64(func, *y)
            }
            _ => None,
        }) else {
            continue;
        };
        let step = func
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .find_map(|i| match i {
                Inst::Bin(v, BinOp::Add, Type::I64, x, _) if *v == back_args[0] && *x == iv => {
                    Some(i)
                }
                _ => None,
            })
            .and_then(|i| match i {
                Inst::Bin(_, _, _, _, y) => func
                    .blocks
                    .iter()
                    .flat_map(|b| &b.insts)
                    .find_map(|j| match j {
                        Inst::Iconst(v, Type::I64, step) if v == y => Some(*step),
                        _ => None,
                    }),
                _ => None,
            })
            .unwrap_or(0);
        let Some(start) = initial_iv(func, header, body, iv) else {
            continue;
        };
        if !header_supported(func, header, body) {
            continue;
        }
        if step <= 0 || start >= bound || (bound - start) % step != 0 {
            continue;
        }
        let trips = (bound - start) / step;
        if trips % factor as i64 != 0 {
            continue;
        }
        let _ = body_args;
        return Some(Plan {
            header,
            body,
            body_args: block.params.clone(),
            back_args: back_args.clone(),
            factor,
        });
    }
    None
}

fn const_i64(func: &Function, mut v: Value) -> Option<i64> {
    for _ in 0..64 {
        if let Some(Inst::Iconst(_, Type::I64, value)) = func
            .blocks
            .iter()
            .flat_map(|b| &b.insts)
            .find(|i| matches!(i, Inst::Iconst(r, Type::I64, _) if *r == v))
        {
            return Some(*value);
        }
        v = match func.blocks.iter().flat_map(|b| &b.insts).find_map(|i| match i {
            Inst::Copy(r, _, src) if *r == v => Some(*src),
            _ => None,
        }) {
            Some(next) => next,
            None => return None,
        };
    }
    None
}

/// Whether an instruction may be executed once per loop iteration without an
/// observable effect or a trap. The RIR unroller skips intermediate header
/// executions, so only pure header computations are safe to leave in place.
fn pure_header_inst(inst: &Inst) -> bool {
    match inst {
        Inst::Nop(..)
        | Inst::Iconst(..)
        | Inst::F32const(..)
        | Inst::F64const(..)
        | Inst::Copy(..)
        | Inst::Icmp(..)
        | Inst::Fcmp(..)
        | Inst::Select(..)
        | Inst::StackAddr(..)
        | Inst::FuncAddr(..)
        | Inst::DynamicStackAddr(..) => true,
        Inst::Bin(_, op, ..) => !matches!(op, BinOp::Sdiv | BinOp::Udiv | BinOp::Srem | BinOp::Urem),
        Inst::Unary(_, op, ..) => !matches!(
            op,
            crate::ir::UnOp::FcvtFromInt
                | crate::ir::UnOp::FcvtToInt
                | crate::ir::UnOp::FcvtFromUint
                | crate::ir::UnOp::FcvtToUint
        ),
        _ => false,
    }
}

/// Reject loops whose body reads a header result that changes with the
/// induction variable, or whose header has a side effect. The rewrite clones
/// only the body, so such a header value would be reused for every unrolled
/// iteration and produce stale results.
fn header_supported(func: &Function, header: usize, body: usize) -> bool {
    let header_block = &func.blocks[header];
    let Some((_, header_insts)) = header_block.insts.split_last() else {
        return false;
    };
    let params: HashSet<Value> = header_block.params.iter().copied().collect();
    let mut header_results: HashMap<Value, Inst> = HashMap::new();
    let mut invariant: HashSet<Value> = HashSet::new();

    // Values defined outside the loop are available and immutable for the whole
    // loop, so they are invariant inputs to the header computation.
    for (bi, block) in func.blocks.iter().enumerate() {
        if bi == header || bi == body {
            continue;
        }
        invariant.extend(block.params.iter().copied());
        for inst in &block.insts {
            invariant.extend(inst.result_values());
        }
    }

    for inst in header_insts {
        if !pure_header_inst(inst) {
            return false;
        }
        let results = inst.result_values();
        for &r in &results {
            header_results.insert(r, inst.clone());
        }
        if inst.operands().iter().all(|v| invariant.contains(v)) {
            invariant.extend(results);
        }
    }

    let Some((_, body_insts)) = func.blocks[body].insts.split_last() else {
        return false;
    };
    for inst in body_insts {
        for v in inst.operands() {
            if params.contains(&v) || invariant.contains(&v) {
                continue;
            }
            if header_results.contains_key(&v) {
                return false;
            }
        }
    }
    true
}

fn initial_iv(func: &Function, header: usize, body: usize, iv: Value) -> Option<i64> {
    let pos = func.blocks[header].params.iter().position(|p| *p == iv)?;
    let preds = crate::cfg_util::predecessors(func);
    let incoming: Vec<usize> =
        preds[header].iter().copied().filter(|&p| p != body).collect();
    // Multiple or conditional entries can have different trip counts. The
    // rewrite has no remainder/entry guard, so only a single unconditional
    // preheader edge is safe.
    if incoming.len() != 1 {
        return None;
    }
    let Some(Inst::Jump(target, args)) = func.blocks[incoming[0]].insts.last() else {
        return None;
    };
    if target.0 as usize != header {
        return None;
    }
    const_i64(func, *args.get(pos)?)
}

fn rewrite(func: &mut Function, plan: Plan) {
    let header_params = func.blocks[plan.header].params.clone();
    let mut clones = Vec::new();
    for _ in 0..plan.factor {
        let b = func.blocks.len() as u32;
        let params: Vec<Value> = plan
            .body_args
            .iter()
            .map(|p| new_value(func, func.ty(*p)))
            .collect();
        func.blocks.push(BlockData { params: params.clone(), insts: Vec::new() });
        clones.push((Block(b), params));
    }

    let mut incoming = plan.body_args.clone();
    for k in 0..plan.factor {
        let (cb, params) = clones[k].clone();
        let mut map = std::collections::HashMap::new();
        for (old, new) in header_params.iter().zip(&params) {
            map.insert(*old, *new);
        }
        let old_insts = func.blocks[plan.body].insts.clone();
        let src_types = func.values.clone();
        let mut insts = Vec::new();
        for inst in old_insts.iter().take(old_insts.len().saturating_sub(1)) {
            if let Some(cloned) = remap_inst(func, &src_types, inst, &mut map) {
                insts.push(cloned);
            }
        }
        let back: Vec<Value> = plan.back_args.iter().map(|v| map_value(&map, *v)).collect();
        if k + 1 < plan.factor {
            insts.push(Inst::Jump(clones[k + 1].0, back.clone()));
        } else {
            insts.push(Inst::Jump(Block(plan.header as u32), back.clone()));
        }
        func.blocks[cb.0 as usize] = BlockData { params, insts };
        if k == 0 {
            let term = func.blocks[plan.header].insts.last_mut().unwrap();
            if let Inst::Brif(_, body_b, body_args, _, _) = term {
                *body_b = cb;
                *body_args = incoming.clone();
            }
        }
        incoming = back;
    }

    // The original body is unreachable now.  Replace it with a valid terminator
    // and prune values that lost their only definition.
    let mut dead_insts = Vec::new();
    let mut dead_vals = Vec::new();
    for ty in func.ret.clone() {
        let v = new_value(func, ty);
        dead_insts.push(zero_const(v, ty));
        dead_vals.push(v);
    }
    dead_insts.push(Inst::Return(dead_vals));
    func.blocks[plan.body] = BlockData { params: Vec::new(), insts: dead_insts };
    let mut defined = HashSet::new();
    for block in &func.blocks {
        defined.extend(block.params.iter().copied());
        for inst in &block.insts {
            defined.extend(inst.result_values());
        }
    }
    let dead_values: HashSet<Value> = func
        .values
        .iter()
        .enumerate()
        .map(|(i, _)| Value(i as u32))
        .filter(|v| !defined.contains(v))
        .collect();
    if !dead_values.is_empty() {
        remove_values(func, &dead_values);
    }
}

pub(crate) fn remap_inst(
    func: &mut Function,
    src_types: &[Type],
    inst: &Inst,
    map: &mut std::collections::HashMap<Value, Value>,
) -> Option<Inst> {
    let out = match inst {
        Inst::Nop(r, ty) => Inst::Nop(new_result(func, src_types, *r, map), *ty),
        Inst::Iconst(r, ty, v) => Inst::Iconst(new_result(func, src_types, *r, map), *ty, *v),
        Inst::F32const(r, v) => Inst::F32const(new_result(func, src_types, *r, map), *v),
        Inst::F64const(r, v) => Inst::F64const(new_result(func, src_types, *r, map), *v),
        Inst::Copy(r, ty, x) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Copy(nr, *ty, map_value(map, *x))
        }
        Inst::Bin(r, op, ty, x, y) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Bin(nr, *op, *ty, map_value(map, *x), map_value(map, *y))
        }
        Inst::Unary(r, op, ty, x) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Unary(nr, *op, *ty, map_value(map, *x))
        }
        Inst::Icmp(r, cc, ty, x, y) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Icmp(nr, *cc, *ty, map_value(map, *x), map_value(map, *y))
        }
        Inst::Fcmp(r, cc, ty, x, y) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Fcmp(nr, *cc, *ty, map_value(map, *x), map_value(map, *y))
        }
        Inst::Select(r, ty, c, x, y) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Select(nr, *ty, map_value(map, *c), map_value(map, *x), map_value(map, *y))
        }
        Inst::StackAddr(r, s, o) => Inst::StackAddr(new_result(func, src_types, *r, map), *s, *o),
        Inst::Load(r, ty, a, o) => {
            let nr = new_result(func, src_types, *r, map);
            Inst::Load(nr, *ty, map_value(map, *a), *o)
        }
        Inst::Store(ty, a, o, v) => {
            Inst::Store(*ty, map_value(map, *a), *o, map_value(map, *v))
        }
        Inst::Call(results, target, args) => {
            let nr = results.iter().map(|r| new_result(func, src_types, *r, map)).collect();
            Inst::Call(nr, *target, args.iter().map(|v| map_value(map, *v)).collect())
        }
        Inst::CallIndirect { results, params, rets, call_conv, callee, args } => {
            let nr = results.iter().map(|r| new_result(func, src_types, *r, map)).collect();
            Inst::CallIndirect {
                results: nr,
                params: params.clone(),
                rets: rets.clone(),
                call_conv: *call_conv,
                callee: map_value(map, *callee),
                args: args.iter().map(|v| map_value(map, *v)).collect(),
            }
        }
        Inst::GlobalValue(r, g) => Inst::GlobalValue(new_result(func, src_types, *r, map), *g),
        Inst::FuncAddr(r, f) => Inst::FuncAddr(new_result(func, src_types, *r, map), *f),
        Inst::DynamicStackAddr(r, s) => Inst::DynamicStackAddr(new_result(func, src_types, *r, map), *s),
        Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..) | Inst::Trap(..) => return None,
        Inst::TryCall { .. } | Inst::TryCallIndirect { .. } => return None,
    };
    Some(out)
}

pub(crate) fn map_value(map: &std::collections::HashMap<Value, Value>, v: Value) -> Value {
    *map.get(&v).unwrap_or(&v)
}

fn new_result(
    func: &mut Function,
    src_types: &[Type],
    old: Value,
    map: &mut std::collections::HashMap<Value, Value>,
) -> Value {
    let ty = src_types[old.0 as usize];
    let v = new_value(func, ty);
    map.insert(old, v);
    v
}

fn zero_const(v: Value, ty: Type) -> Inst {
    match ty {
        Type::F32 => Inst::F32const(v, 0.0),
        Type::F64 => Inst::F64const(v, 0.0),
        Type::Bool => Inst::Iconst(v, Type::Bool, 0),
        _ => Inst::Iconst(v, ty, 0),
    }
}

fn new_value(func: &mut Function, ty: Type) -> Value {
    let v = Value(func.values.len() as u32);
    func.values.push(ty);
    v
}
