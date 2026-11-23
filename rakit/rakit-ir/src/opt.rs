//! A first optimizer over Rakit IR: constant folding, branch folding and dead
//! code elimination. Values keep stable indices, so dead instructions become
//! [`Inst::Nop`] and folding uses [`Inst::Copy`] instead of renumbering.

use std::collections::HashMap;

use crate::int_sem;
use crate::ir::{BinOp, FloatCc, Function, Inst, IntCc, Type, Value};

#[derive(Clone, Copy)]
enum Const {
    I(i64),
    F32(f32),
    F64(f64),
}

fn const_map(func: &Function) -> HashMap<Value, Const> {
    let mut map = HashMap::new();
    for blk in &func.blocks {
        for inst in &blk.insts {
            match inst {
                Inst::Iconst(r, ty, v) => {
                    map.insert(*r, Const::I(int_sem::wrap(*ty, *v)));
                }
                Inst::F32const(r, v) => {
                    map.insert(*r, Const::F32(*v));
                }
                Inst::F64const(r, v) => {
                    map.insert(*r, Const::F64(*v));
                }
                _ => {}
            }
        }
    }
    map
}

fn eval_bin(op: BinOp, ty: Type, a: Const, b: Const) -> Option<Const> {
    match (ty, a, b) {
        (Type::F32, Const::F32(x), Const::F32(y)) => {
            let r = match op {
                BinOp::Fadd => x + y,
                BinOp::Fsub => x - y,
                BinOp::Fmul => x * y,
                BinOp::Fdiv => x / y,
                _ => return None,
            };
            Some(Const::F32(r))
        }
        (Type::F64, Const::F64(x), Const::F64(y)) => {
            let r = match op {
                BinOp::Fadd => x + y,
                BinOp::Fsub => x - y,
                BinOp::Fmul => x * y,
                BinOp::Fdiv => x / y,
                _ => return None,
            };
            Some(Const::F64(r))
        }
        (ty, Const::I(x), Const::I(y)) if ty.is_int() => {
            int_sem::eval_bin(op, ty, x, y).map(Const::I)
        }
        _ => None,
    }
}

fn eval_fcmp(cc: FloatCc, a: f64, b: f64) -> bool {
    match cc {
        FloatCc::Eq => a == b,
        FloatCc::Ne => a != b,
        FloatCc::Lt => a < b,
        FloatCc::Le => a <= b,
        FloatCc::Gt => a > b,
        FloatCc::Ge => a >= b,
    }
}

fn const_inst(r: Value, ty: Type, c: Const) -> Option<Inst> {
    match c {
        Const::I(v) => Some(Inst::Iconst(r, ty, v)),
        Const::F32(v) => Some(Inst::F32const(r, v)),
        Const::F64(v) => Some(Inst::F64const(r, v)),
    }
}

fn fold(func: &mut Function) -> bool {
    let consts = const_map(func);
    let mut changed = false;
    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            let inst = func.blocks[bi].insts[ii].clone();
            let replacement = match inst {
                Inst::Bin(r, op, ty, x, y) => {
                    if x == y && ty.is_int() {
                        match op {
                            BinOp::Sub | BinOp::Xor => Some(Inst::Iconst(r, ty, 0)),
                            BinOp::And | BinOp::Or => Some(Inst::Copy(r, ty, x)),
                            _ => None,
                        }
                    } else {
                        let cx = consts.get(&x).copied();
                        let cy = consts.get(&y).copied();
                        match (cx, cy) {
                            (Some(a), Some(b)) => {
                                eval_bin(op, ty, a, b).and_then(|c| const_inst(r, ty, c))
                            }
                            (None, Some(Const::I(0))) => match op {
                                BinOp::Add | BinOp::Sub | BinOp::Or | BinOp::Xor | BinOp::Shl
                                | BinOp::Shr | BinOp::Ushr => Some(Inst::Copy(r, ty, x)),
                                BinOp::Mul => Some(Inst::Iconst(r, ty, 0)),
                                BinOp::And => Some(Inst::Iconst(r, ty, 0)),
                                _ => None,
                            },
                            (Some(Const::I(0)), None) => match op {
                                BinOp::Add | BinOp::Or | BinOp::Xor => Some(Inst::Copy(r, ty, y)),
                                BinOp::Mul | BinOp::And => Some(Inst::Iconst(r, ty, 0)),
                                _ => None,
                            },
                            (None, Some(Const::I(1))) if op == BinOp::Mul => {
                                Some(Inst::Copy(r, ty, x))
                            }
                            _ => None,
                        }
                    }
                }
                Inst::Icmp(r, cc, ty, x, y) => {
                    if x == y && ty.is_int() {
                        let v = matches!(
                            cc,
                            IntCc::Eq
                                | IntCc::Sle
                                | IntCc::Sge
                                | IntCc::Ule
                                | IntCc::Uge
                        );
                        Some(Inst::Iconst(r, Type::Bool, v as i64))
                    } else {
                        match (consts.get(&x), consts.get(&y)) {
                            (Some(Const::I(a)), Some(Const::I(b))) => {
                                Some(Inst::Iconst(
                                    r,
                                    Type::Bool,
                                    int_sem::eval_icmp(cc, ty, *a, *b) as i64,
                                ))
                            }
                            _ => None,
                        }
                    }
                }
                Inst::Fcmp(r, cc, _ty, x, y) => {
                    let a = match consts.get(&x) {
                        Some(Const::F32(v)) => Some(*v as f64),
                        Some(Const::F64(v)) => Some(*v),
                        _ => None,
                    };
                    let b = match consts.get(&y) {
                        Some(Const::F32(v)) => Some(*v as f64),
                        Some(Const::F64(v)) => Some(*v),
                        _ => None,
                    };
                    match (a, b) {
                        (Some(a), Some(b)) => {
                            Some(Inst::Iconst(r, Type::Bool, eval_fcmp(cc, a, b) as i64))
                        }
                        _ => None,
                    }
                }
                Inst::Select(r, ty, c, x, y) => {
                    if x == y {
                        Some(Inst::Copy(r, ty, x))
                    } else {
                        match consts.get(&c) {
                            Some(Const::I(v)) => {
                                Some(Inst::Copy(r, ty, if *v != 0 { x } else { y }))
                            }
                            _ => None,
                        }
                    }
                }
                Inst::Copy(r, ty, x) => match consts.get(&x) {
                    Some(c) => const_inst(r, ty, *c),
                    None => None,
                },
                Inst::Brif(c, t, targs, f, fargs) => {
                    if t == f && targs == fargs {
                        Some(Inst::Jump(t, targs))
                    } else {
                        match consts.get(&c) {
                            Some(Const::I(v)) => {
                                if *v != 0 {
                                    Some(Inst::Jump(t, targs))
                                } else {
                                    Some(Inst::Jump(f, fargs))
                                }
                            }
                            _ => None,
                        }
                    }
                }
                _ => None,
            };
            if let Some(new) = replacement {
                func.blocks[bi].insts[ii] = new;
                changed = true;
            }
        }
    }
    changed
}

fn dce(func: &mut Function) -> bool {
    let mut uses: HashMap<Value, usize> = HashMap::new();
    for blk in &func.blocks {
        for inst in &blk.insts {
            for u in inst.operands() {
                *uses.entry(u).or_insert(0) += 1;
            }
        }
    }

    let mut changed = false;
    for bi in 0..func.blocks.len() {
        for ii in 0..func.blocks[bi].insts.len() {
            let inst = &func.blocks[bi].insts[ii];
            if matches!(inst, Inst::Load(..) | Inst::Store(..) | Inst::Call(..)) {
                continue;
            }
            if let Some(r) = inst.result_value() {
                if !matches!(inst, Inst::Nop(..)) && uses.get(&r).copied().unwrap_or(0) == 0 {
                    let ty = func.values[r.0 as usize];
                    func.blocks[bi].insts[ii] = Inst::Nop(r, ty);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Run the optimizer to a fixpoint. Returns whether anything changed.
pub fn optimize(func: &mut Function) -> bool {
    let mut any = false;
    for _ in 0..1000 {
        let mut changed = false;
        changed |= fold(func);
        changed |= dce(func);
        if !changed {
            break;
        }
        any = true;
    }
    any
}
