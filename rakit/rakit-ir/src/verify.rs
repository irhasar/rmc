//! SSA verifier for Rakit IR: single definition, dominance and types.

use std::collections::HashSet;

use crate::ir::{Block, Function, Inst, Type, Value};

#[derive(Clone, Copy)]
struct Def {
    block: Block,
    index: usize,
    is_param: bool,
}

pub fn verify(func: &Function) -> Result<(), Vec<String>> {
    let mut errs = Vec::new();
    let n = func.num_blocks();
    if n == 0 {
        errs.push("function has no blocks".to_string());
        return Err(errs);
    }
    if func.entry.0 as usize >= n {
        errs.push("entry block out of range".to_string());
        return Err(errs);
    }

    let mut defs: Vec<Option<Def>> = vec![None; func.values.len()];

    let mut define = |errs: &mut Vec<String>, v: Value, d: Def| {
        if v.0 as usize >= defs.len() {
            errs.push(format!("value {v:?} out of range"));
            return;
        }
        if defs[v.0 as usize].is_some() {
            errs.push(format!("value {v:?} is defined more than once"));
        }
        defs[v.0 as usize] = Some(d);
    };

    for (bi, blk) in func.blocks.iter().enumerate() {
        let b = Block(bi as u32);
        for (pi, p) in blk.params.iter().enumerate() {
            define(&mut errs, *p, Def { block: b, index: pi, is_param: true });
        }
        if blk.insts.is_empty() {
            errs.push(format!("block {b:?} has no terminator"));
        }
        for (ii, inst) in blk.insts.iter().enumerate() {
            let last = ii + 1 == blk.insts.len();
            if inst.is_terminator() && !last {
                errs.push(format!("block {b:?} has a terminator before its end"));
            }
            if !inst.is_terminator() && last {
                errs.push(format!("block {b:?} does not end in a terminator"));
            }
            if let Some(r) = inst.result_value() {
                define(&mut errs, r, Def { block: b, index: ii, is_param: false });
            }
        }
    }

    for (vi, d) in defs.iter().enumerate() {
        if d.is_none() {
            errs.push(format!("value v{vi} is never defined"));
        }
    }

    let dom = dominators(func);

    let ty_of = |v: Value| -> Option<Type> {
        func.values.get(v.0 as usize).copied()
    };

    for (bi, blk) in func.blocks.iter().enumerate() {
        let b = Block(bi as u32);
        for (ii, inst) in blk.insts.iter().enumerate() {
            for u in inst.operands() {
                match defs.get(u.0 as usize).copied().flatten() {
                    None => errs.push(format!("use of undefined value {u:?}")),
                    Some(d) => {
                        if d.block != b {
                            if !dominates(&dom, d.block, b) {
                                errs.push(format!(
                                    "use of {u:?} in {b:?} is not dominated by its definition in {:?}",
                                    d.block
                                ));
                            }
                        } else if !d.is_param && d.index >= ii {
                            errs.push(format!(
                                "use of {u:?} in {b:?} before its definition"
                            ));
                        }
                    }
                }
            }

            let check_same = |errs: &mut Vec<String>, x: Value, y: Value| {
                if let (Some(tx), Some(ty)) = (ty_of(x), ty_of(y)) {
                    if tx != ty {
                        errs.push(format!("operand types differ: {x:?}:{tx:?} vs {y:?}:{ty:?}"));
                    }
                }
            };

            match inst {
                Inst::Nop(_, _) => {}
                Inst::Iconst(r, ty, _) => {
                    if !ty.is_int() && !ty.is_bool() {
                        errs.push(format!("integer constant has non-integer type {ty:?}"));
                    }
                    if ty_of(*r) != Some(*ty) {
                        errs.push(format!("result {r:?} type mismatch"));
                    }
                }
                Inst::F32const(r, _) => {
                    if ty_of(*r) != Some(Type::F32) {
                        errs.push(format!("f32 constant result {r:?} is not F32"));
                    }
                }
                Inst::F64const(r, _) => {
                    if ty_of(*r) != Some(Type::F64) {
                        errs.push(format!("f64 constant result {r:?} is not F64"));
                    }
                }
                Inst::Copy(r, ty, x) => {
                    if ty_of(*x) != Some(*ty) {
                        errs.push(format!("copy operand type mismatch for {r:?}"));
                    }
                    if ty_of(*r) != Some(*ty) {
                        errs.push(format!("copy result {r:?} type mismatch"));
                    }
                }
                Inst::Bin(r, op, ty, x, y) => {
                    check_same(&mut errs, *x, *y);
                    if ty_of(*x) != Some(*ty) {
                        errs.push(format!("binary operand type mismatch for {r:?}"));
                    }
                    if op.is_float() {
                        if !ty.is_float() {
                            errs.push(format!("float op {op:?} on non-float type {ty:?}"));
                        }
                    } else if !ty.is_int() {
                        errs.push(format!("integer op {op:?} on non-integer type {ty:?}"));
                    }
                    if ty_of(*r) != Some(*ty) {
                        errs.push(format!("binary result {r:?} type mismatch"));
                    }
                }
                Inst::Icmp(r, _, ty, x, y) => {
                    check_same(&mut errs, *x, *y);
                    if !ty.is_int() {
                        errs.push(format!("icmp on non-integer type {ty:?}"));
                    }
                    if ty_of(*r) != Some(Type::Bool) {
                        errs.push(format!("icmp result {r:?} is not Bool"));
                    }
                }
                Inst::Fcmp(r, _, ty, x, y) => {
                    check_same(&mut errs, *x, *y);
                    if !ty.is_float() {
                        errs.push(format!("fcmp on non-float type {ty:?}"));
                    }
                    if ty_of(*r) != Some(Type::Bool) {
                        errs.push(format!("fcmp result {r:?} is not Bool"));
                    }
                }
                Inst::Select(r, ty, c, x, y) => {
                    check_same(&mut errs, *x, *y);
                    if ty_of(*c) != Some(Type::Bool) {
                        errs.push(format!("select condition {c:?} is not Bool"));
                    }
                    if ty_of(*x) != Some(*ty) || ty_of(*r) != Some(*ty) {
                        errs.push(format!("select {r:?} type mismatch"));
                    }
                }
                Inst::StackAddr(r, slot, _) => {
                    if slot.0 as usize >= func.stack_slots.len() {
                        errs.push(format!("stack slot {slot:?} out of range"));
                    }
                    if ty_of(*r) != Some(Type::I64) {
                        errs.push(format!("stack address {r:?} is not I64"));
                    }
                }
                Inst::Load(r, ty, addr, _) => {
                    if !ty_of(*addr).map(|t| t.is_int()).unwrap_or(false) {
                        errs.push(format!("load address {addr:?} is not an integer pointer"));
                    }
                    if ty_of(*r) != Some(*ty) {
                        errs.push(format!("load result {r:?} type mismatch"));
                    }
                }
                Inst::Store(ty, addr, _, value) => {
                    if !ty_of(*addr).map(|t| t.is_int()).unwrap_or(false) {
                        errs.push(format!("store address {addr:?} is not an integer pointer"));
                    }
                    if ty_of(*value) != Some(*ty) {
                        errs.push(format!("store value {value:?} type mismatch"));
                    }
                }
                Inst::Jump(t, args) => {
                    check_args(&mut errs, func, *t, args, ty_of);
                }
                Inst::Brif(c, t, targs, f, fargs) => {
                    if ty_of(*c) != Some(Type::Bool) {
                        errs.push(format!("brif condition {c:?} is not Bool"));
                    }
                    check_args(&mut errs, func, *t, targs, ty_of);
                    check_args(&mut errs, func, *f, fargs, ty_of);
                }
                Inst::Return(args) => {
                    if args.len() != func.ret.len() {
                        errs.push("return arity mismatch".to_string());
                    } else {
                        for (a, ty) in args.iter().zip(&func.ret) {
                            if ty_of(*a) != Some(*ty) {
                                errs.push(format!("return value {a:?} type mismatch"));
                            }
                        }
                    }
                }
            }
        }
    }

    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn check_args(
    errs: &mut Vec<String>,
    func: &Function,
    target: Block,
    args: &[Value],
    ty_of: impl Fn(Value) -> Option<Type>,
) {
    if target.0 as usize >= func.num_blocks() {
        errs.push(format!("branch target {target:?} out of range"));
        return;
    }
    let params = &func.block(target).params;
    if params.len() != args.len() {
        errs.push(format!("branch to {target:?} passes {} args for {} params", args.len(), params.len()));
        return;
    }
    for (a, p) in args.iter().zip(params) {
        let pt = ty_of(*p);
        if ty_of(*a) != pt {
            errs.push(format!("branch arg {a:?} does not match param {p:?} type"));
        }
    }
}

fn dominators(func: &Function) -> Vec<HashSet<Block>> {
    let n = func.num_blocks();
    let mut preds = vec![Vec::new(); n];
    for (bi, blk) in func.blocks.iter().enumerate() {
        if let Some(term) = blk.terminator() {
            match term {
                Inst::Jump(t, _) => preds[t.0 as usize].push(Block(bi as u32)),
                Inst::Brif(_, t, _, f, _) => {
                    preds[t.0 as usize].push(Block(bi as u32));
                    preds[f.0 as usize].push(Block(bi as u32));
                }
                _ => {}
            }
        }
    }

    let all: HashSet<Block> = (0..n).map(|i| Block(i as u32)).collect();
    let mut dom: Vec<HashSet<Block>> = vec![all; n];
    dom[func.entry.0 as usize] = HashSet::from([func.entry]);

    loop {
        let mut changed = false;
        for b in 0..n {
            if Block(b as u32) == func.entry {
                continue;
            }
            let mut new: HashSet<Block> = if preds[b].is_empty() {
                HashSet::from([Block(b as u32)])
            } else {
                let mut it = preds[b].iter();
                let first = dom[it.next().unwrap().0 as usize].clone();
                it.fold(first, |acc, p| acc.intersection(&dom[p.0 as usize]).copied().collect())
            };
            new.insert(Block(b as u32));
            if new != dom[b] {
                dom[b] = new;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    dom
}

fn dominates(dom: &[HashSet<Block>], a: Block, b: Block) -> bool {
    dom[b.0 as usize].contains(&a)
}
