//! RIR port of the self-recursive linear-recurrence rewrite.
//!
//! `F(x) = if x < base { x } else { F(x - c1) + F(x - c2) }` becomes
//! `acc = 0; while x >= base { acc += F(x - c1); x -= c2 } return acc + x`.
//! The matcher is deliberately exact: the function must call itself directly
//! twice, both calls must subtract a constant from the same induction value,
//! and the results must be added.

use std::collections::HashSet;

use crate::cfg_util::remove_values;
use crate::ir::{BinOp, BlockData, Function, Inst, IntCc, Type, Value};

pub fn run(func: &mut Function) -> bool {
    let Some(p) = try_match(func) else {
        return false;
    };
    rewrite(func, p);
    true
}

struct Pat {
    n: Value,
    base: i64,
    c1: i64,
    c2: i64,
    target: u32,
}

fn try_match(func: &Function) -> Option<Pat> {
    if func.blocks.len() != 3 || func.entry.0 != 0 {
        return None;
    }
    let entry = &func.blocks[0];
    if entry.params.len() != 1 {
        return None;
    }
    let n = entry.params[0];
    if func.ty(n) != Type::I64 {
        return None;
    }
    let [Inst::Iconst(_c, ty, base), Inst::Icmp(cond, IntCc::Ult, cmp_ty, x, y), Inst::Brif(
        c,
        then_b,
        then_args,
        else_b,
        else_args,
    )] = entry.insts.as_slice()
    else {
        return None;
    };
    if *ty != Type::I64 || *cmp_ty != Type::I64 || *x != n || *y != *_c || *c != *cond {
        return None;
    }
    if !then_args.is_empty() || !else_args.is_empty() {
        return None;
    }
    let base_block = func.blocks.get(then_b.0 as usize)?;
    let rec_block = func.blocks.get(else_b.0 as usize)?;
    let [Inst::Return(args)] = base_block.insts.as_slice() else {
        return None;
    };
    if args.as_slice() != [n] {
        return None;
    }

    let mut i = 0;
    let insts = &rec_block.insts;
    let (_n1, t1, target, c1) = call_sub(insts, &mut i, n)?;
    let (_n2, t2, target2, c2) = call_sub(insts, &mut i, n)?;
    if target != target2 {
        return None;
    }
    let self_target = func.self_target.as_ref()?;
    if func.externals.get(target as usize)?.target != *self_target {
        return None;
    }
    let Inst::Bin(sum, BinOp::Add, Type::I64, a, b) = insts.get(i)? else {
        return None;
    };
    let Inst::Return(ret) = insts.get(i + 1)? else {
        return None;
    };
    if !((*a == t1 && *b == t2) || (*a == t2 && *b == t1)) || ret.as_slice() != [*sum] {
        return None;
    }
    if i + 2 != insts.len() {
        return None;
    }
    Some(Pat { n, base: *base, c1, c2, target })
}

fn call_sub(insts: &[Inst], i: &mut usize, n: Value) -> Option<(Value, Value, u32, i64)> {
    let Inst::Iconst(c, Type::I64, constant) = insts.get(*i)? else { return None };
    let Inst::Bin(nv, BinOp::Sub, Type::I64, x, y) = insts.get(*i + 1)? else { return None };
    if *x != n || *y != *c {
        return None;
    }
    let Inst::Call(results, target, args) = insts.get(*i + 2)? else { return None };
    if results.len() != 1 || args.as_slice() != [*nv] {
        return None;
    }
    *i += 3;
    Some((*nv, results[0], *target, *constant))
}

fn rewrite(func: &mut Function, p: Pat) {
    let old_n = p.n;
    let loop_b = func.blocks.len() as u32;
    let base_b = loop_b + 1;
    let rec_b = loop_b + 2;
    func.blocks.push(BlockData::default());
    func.blocks.push(BlockData::default());
    func.blocks.push(BlockData::default());

    let loop_n = new_value(func, Type::I64);
    let loop_acc = new_value(func, Type::I64);
    let base_n = new_value(func, Type::I64);
    let base_acc = new_value(func, Type::I64);
    let rec_n = new_value(func, Type::I64);
    let rec_acc = new_value(func, Type::I64);
    func.blocks[loop_b as usize].params = vec![loop_n, loop_acc];
    func.blocks[base_b as usize].params = vec![base_n, base_acc];
    func.blocks[rec_b as usize].params = vec![rec_n, rec_acc];

    let zero = new_value(func, Type::I64);
    let cbase = new_value(func, Type::I64);
    let cond = new_value(func, Type::Bool);
    let cn1 = new_value(func, Type::I64);
    let cn2 = new_value(func, Type::I64);
    let n1 = new_value(func, Type::I64);
    let n2 = new_value(func, Type::I64);
    let t1 = new_value(func, Type::I64);
    let acc1 = new_value(func, Type::I64);
    let sum = new_value(func, Type::I64);

    func.blocks[0].insts = vec![
        Inst::Iconst(zero, Type::I64, 0),
        Inst::Jump(crate::ir::Block(loop_b), vec![old_n, zero]),
    ];
    func.blocks[loop_b as usize].insts = vec![
        Inst::Iconst(cbase, Type::I64, p.base),
        Inst::Icmp(cond, IntCc::Ult, Type::I64, loop_n, cbase),
        Inst::Brif(
            cond,
            crate::ir::Block(base_b),
            vec![loop_n, loop_acc],
            crate::ir::Block(rec_b),
            vec![loop_n, loop_acc],
        ),
    ];
    func.blocks[base_b as usize].insts = vec![
        Inst::Bin(sum, BinOp::Add, Type::I64, base_n, base_acc),
        Inst::Return(vec![sum]),
    ];
    func.blocks[rec_b as usize].insts = vec![
        Inst::Iconst(cn1, Type::I64, p.c1),
        Inst::Bin(n1, BinOp::Sub, Type::I64, rec_n, cn1),
        Inst::Call(vec![t1], p.target, vec![n1]),
        Inst::Iconst(cn2, Type::I64, p.c2),
        Inst::Bin(n2, BinOp::Sub, Type::I64, rec_n, cn2),
        Inst::Bin(acc1, BinOp::Add, Type::I64, rec_acc, t1),
        Inst::Jump(crate::ir::Block(loop_b), vec![n2, acc1]),
    ];

    // The old base/rec blocks are now unreachable.  Keep them as valid
    // terminators and drop the values that no longer have a definition.
    let dead1 = new_value(func, Type::I64);
    let dead2 = new_value(func, Type::I64);
    func.blocks[1].insts = vec![
        Inst::Iconst(dead1, Type::I64, 0),
        Inst::Return(vec![dead1]),
    ];
    func.blocks[2].insts = vec![
        Inst::Iconst(dead2, Type::I64, 0),
        Inst::Return(vec![dead2]),
    ];
    func.blocks[1].params.clear();
    func.blocks[2].params.clear();
    let mut defined = HashSet::new();
    for block in &func.blocks {
        defined.extend(block.params.iter().copied());
        for inst in &block.insts {
            defined.extend(inst.result_values());
        }
    }
    let dead: HashSet<Value> = func
        .values
        .iter()
        .enumerate()
        .map(|(i, _)| Value(i as u32))
        .filter(|v| !defined.contains(v))
        .collect();
    if !dead.is_empty() {
        remove_values(func, &dead);
    }
}

fn new_value(func: &mut Function, ty: Type) -> Value {
    let v = Value(func.values.len() as u32);
    func.values.push(ty);
    v
}
