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
            for r in inst.result_values() {
                define(&mut errs, r, Def { block: b, index: ii, is_param: false });
            }
        }
    }

    for (vi, d) in defs.iter().enumerate() {
        if d.is_none() {
            errs.push(format!("value v{vi} is never defined"));
        }
    }

    let reachable = reachable_blocks(func);
    let dom = dominators(func);

    let ty_of = |v: Value| -> Option<Type> { func.values.get(v.0 as usize).copied() };

    for (bi, blk) in func.blocks.iter().enumerate() {
        let b = Block(bi as u32);
        for (ii, inst) in blk.insts.iter().enumerate() {
            for u in inst.operands() {
                match defs.get(u.0 as usize).copied().flatten() {
                    None => errs.push(format!("use of undefined value {u:?}")),
                    Some(d) => {
                        if d.block != b {
                            if reachable[bi]
                                && reachable[d.block.0 as usize]
                                && !dominates(&dom, d.block, b)
                            {
                                errs.push(format!(
                                    "use of {u:?} in {b:?} is not dominated by its definition in {:?}",
                                    d.block
                                ));
                            }
                        } else if !d.is_param && d.index >= ii {
                            errs.push(format!("use of {u:?} in {b:?} before its definition"));
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
                Inst::Trap(code) => {
                    if *code == 0 {
                        errs.push("trap code must be non-zero".to_string());
                    }
                }
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
                Inst::Unary(r, _, ty, x) => {
                    if ty_of(*x).is_none() {
                        errs.push(format!("unary operand {x:?} is undefined"));
                    }
                    if ty_of(*r) != Some(*ty) {
                        errs.push(format!("unary result {r:?} type mismatch"));
                    }
                }
                Inst::Icmp(r, _, ty, x, y) => {
                    check_same(&mut errs, *x, *y);
                    if ty_of(*x) != Some(*ty) || ty_of(*y) != Some(*ty) {
                        errs.push(format!("icmp operand type differs from declared {ty:?}"));
                    }
                    if !ty.is_int() {
                        errs.push(format!("icmp on non-integer type {ty:?}"));
                    }
                    if ty_of(*r) != Some(Type::Bool) {
                        errs.push(format!("icmp result {r:?} is not Bool"));
                    }
                }
                Inst::Fcmp(r, _, ty, x, y) => {
                    check_same(&mut errs, *x, *y);
                    if ty_of(*x) != Some(*ty) || ty_of(*y) != Some(*ty) {
                        errs.push(format!("fcmp operand type differs from declared {ty:?}"));
                    }
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
                Inst::Call(results, target, args) => {
                    let Some(ext) = func.externals.get(*target as usize) else {
                        errs.push(format!("call target {target} out of range"));
                        continue;
                    };
                    if args.len() != ext.params.len() {
                        errs.push("call argument arity mismatch".to_string());
                    } else {
                        for (arg, ty) in args.iter().zip(&ext.params) {
                            if ty_of(*arg) != Some(*ty) {
                                errs.push(format!("call argument {arg:?} type mismatch"));
                            }
                        }
                    }
                    if results.len() != ext.rets.len() {
                        errs.push("call result arity mismatch".to_string());
                    } else {
                        for (r, ty) in results.iter().zip(&ext.rets) {
                            if ty_of(*r) != Some(*ty) {
                                errs.push(format!("call result {r:?} type mismatch"));
                            }
                        }
                    }
                }
                Inst::CallIndirect { results, params, rets, callee, args, .. } => {
                    if !ty_of(*callee).map(|t| matches!(t, Type::Ptr | Type::I64)).unwrap_or(false)
                    {
                        errs.push(format!("indirect callee {callee:?} is not a pointer"));
                    }
                    if args.len() != params.len() {
                        errs.push("indirect call argument arity mismatch".to_string());
                    } else {
                        for (arg, ty) in args.iter().zip(params) {
                            if ty_of(*arg) != Some(*ty) {
                                errs.push(format!("indirect argument {arg:?} type mismatch"));
                            }
                        }
                    }
                    if results.len() != rets.len() {
                        errs.push("indirect call result arity mismatch".to_string());
                    } else {
                        for (r, ty) in results.iter().zip(rets) {
                            if ty_of(*r) != Some(*ty) {
                                errs.push(format!("indirect result {r:?} type mismatch"));
                            }
                        }
                    }
                }
                Inst::GlobalValue(r, index) => {
                    if *index as usize >= func.globals.len() {
                        errs.push(format!("global value {index} out of range"));
                    }
                    if ty_of(*r) != Some(Type::I64) {
                        errs.push(format!("global value result {r:?} is not I64"));
                    }
                }
                Inst::FuncAddr(r, target) => {
                    if *target as usize >= func.externals.len() {
                        errs.push(format!("function address target {target} out of range"));
                    }
                    if ty_of(*r) != Some(Type::I64) {
                        errs.push(format!("function address result {r:?} is not I64"));
                    }
                }
                Inst::DynamicStackAddr(r, slot) => {
                    if *slot as usize >= func.dynamic_stack_slots.len() {
                        errs.push(format!("dynamic stack slot {slot} out of range"));
                    }
                    if ty_of(*r) != Some(Type::I64) {
                        errs.push(format!("dynamic stack address {r:?} is not I64"));
                    }
                }
                Inst::TryCall { results, target, args, normal, exception } => {
                    let Some(ext) = func.externals.get(*target as usize) else {
                        errs.push(format!("try-call target {target} out of range"));
                        continue;
                    };
                    check_call_args(&mut errs, args, &ext.params, ty_of);
                    if !results.is_empty() {
                        errs.push("try-call has direct SSA results".to_string());
                    }
                    check_try_call(&mut errs, func, normal, &ext.rets, ty_of);
                    check_exception(&mut errs, func, exception, &ext.rets, ty_of);
                }
                Inst::TryCallIndirect {
                    results,
                    params,
                    rets,
                    callee,
                    args,
                    normal,
                    exception,
                    ..
                } => {
                    if !ty_of(*callee).map(|t| matches!(t, Type::Ptr | Type::I64)).unwrap_or(false)
                    {
                        errs.push(format!("indirect callee {callee:?} is not a pointer"));
                    }
                    check_call_args(&mut errs, args, params, ty_of);
                    if !results.is_empty() {
                        errs.push("indirect try-call has direct SSA results".to_string());
                    }
                    check_try_call(&mut errs, func, normal, rets, ty_of);
                    check_exception(&mut errs, func, exception, rets, ty_of);
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

fn check_call_args(
    errs: &mut Vec<String>,
    args: &[Value],
    params: &[Type],
    ty_of: impl Fn(Value) -> Option<Type>,
) {
    if args.len() != params.len() {
        errs.push("call argument arity mismatch".to_string());
        return;
    }
    for (arg, ty) in args.iter().zip(params) {
        if ty_of(*arg) != Some(*ty) {
            errs.push(format!("call argument {arg:?} type mismatch"));
        }
    }
}

fn check_try_call(
    errs: &mut Vec<String>,
    func: &Function,
    call: &crate::ir::TryBlockCall,
    rets: &[Type],
    ty_of: impl Fn(Value) -> Option<Type>,
) {
    if call.block.0 as usize >= func.num_blocks() {
        errs.push(format!("try-call target {:?} out of range", call.block));
        return;
    }
    let params = &func.block(call.block).params;
    if params.len() != call.args.len() {
        errs.push("try-call block argument arity mismatch".to_string());
        return;
    }
    for (arg, param) in call.args.iter().zip(params) {
        let expected = match arg {
            crate::ir::TryArg::Value(v) => ty_of(*v),
            crate::ir::TryArg::Ret(i) => rets.get(*i as usize).copied(),
            crate::ir::TryArg::Exn(_) => Some(Type::I64),
        };
        if expected != ty_of(*param) {
            errs.push(format!("try-call block argument {arg:?} type mismatch"));
        }
    }
}

fn check_exception(
    errs: &mut Vec<String>,
    func: &Function,
    exception: &[crate::ir::ExceptionItem],
    rets: &[Type],
    ty_of: impl Fn(Value) -> Option<Type>,
) {
    for item in exception {
        match item {
            crate::ir::ExceptionItem::Tag(_, call) | crate::ir::ExceptionItem::Default(call) => {
                check_try_call(errs, func, call, rets, &ty_of)
            }
            crate::ir::ExceptionItem::Context(v) => {
                if ty_of(*v).is_none() {
                    errs.push(format!("exception context {v:?} has no type"));
                }
            }
        }
    }
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
        errs.push(format!(
            "branch to {target:?} passes {} args for {} params",
            args.len(),
            params.len()
        ));
        return;
    }
    for (a, p) in args.iter().zip(params) {
        let pt = ty_of(*p);
        if ty_of(*a) != pt {
            errs.push(format!("branch arg {a:?} does not match param {p:?} type"));
        }
    }
}

fn reachable_blocks(func: &Function) -> Vec<bool> {
    let n = func.num_blocks();
    let mut seen = vec![false; n];
    if func.entry.0 as usize >= n {
        return seen;
    }
    let mut stack = vec![func.entry.0 as usize];
    seen[func.entry.0 as usize] = true;
    while let Some(b) = stack.pop() {
        let Some(term) = func.blocks[b].terminator() else { continue };
        let mut push = |target: Block| {
            if (target.0 as usize) < n && !seen[target.0 as usize] {
                seen[target.0 as usize] = true;
                stack.push(target.0 as usize);
            }
        };
        match term {
            Inst::Jump(t, _) => push(*t),
            Inst::Brif(_, t, _, f, _) => {
                push(*t);
                push(*f);
            }
            Inst::TryCall { normal, exception, .. }
            | Inst::TryCallIndirect { normal, exception, .. } => {
                push(normal.block);
                for item in exception {
                    match item {
                        crate::ir::ExceptionItem::Tag(_, call)
                        | crate::ir::ExceptionItem::Default(call) => push(call.block),
                        crate::ir::ExceptionItem::Context(_) => {}
                    }
                }
            }
            _ => {}
        }
    }
    seen
}

fn dominators(func: &Function) -> Vec<HashSet<Block>> {
    let n = func.num_blocks();
    let reachable = reachable_blocks(func);
    let mut preds = vec![Vec::new(); n];
    for (bi, blk) in func.blocks.iter().enumerate() {
        if !reachable[bi] {
            continue;
        }
        if let Some(term) = blk.terminator() {
            match term {
                Inst::Jump(t, _) => {
                    if let Some(p) = preds.get_mut(t.0 as usize) {
                        p.push(Block(bi as u32));
                    }
                }
                Inst::Brif(_, t, _, f, _) => {
                    for target in [t, f] {
                        if let Some(p) = preds.get_mut(target.0 as usize) {
                            p.push(Block(bi as u32));
                        }
                    }
                }
                Inst::TryCall { normal, exception, .. }
                | Inst::TryCallIndirect { normal, exception, .. } => {
                    if let Some(p) = preds.get_mut(normal.block.0 as usize) {
                        p.push(Block(bi as u32));
                    }
                    for item in exception {
                        match item {
                            crate::ir::ExceptionItem::Tag(_, call)
                            | crate::ir::ExceptionItem::Default(call) => {
                                if let Some(p) = preds.get_mut(call.block.0 as usize) {
                                    p.push(Block(bi as u32));
                                }
                            }
                            crate::ir::ExceptionItem::Context(_) => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let all: HashSet<Block> = (0..n).map(|i| Block(i as u32)).collect();
    let mut dom: Vec<HashSet<Block>> = vec![all; n];
    if (func.entry.0 as usize) < n {
        dom[func.entry.0 as usize] = HashSet::from([func.entry]);
    }

    loop {
        let mut changed = false;
        for b in 0..n {
            if !reachable[b] || Block(b as u32) == func.entry {
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
