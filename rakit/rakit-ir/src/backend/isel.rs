//! RIR to machine-IR instruction selection for the integer subset.

use std::collections::HashMap;

use crate::backend::mach::{MachBinOp, MachFunction, MachInst, Operand, VReg};
use crate::backend::{BackendError, InstructionSelector};
use crate::ir::{BinOp, Function, Inst, IntCc, Type, Value};

pub struct RirSelector;

impl InstructionSelector for RirSelector {
    fn select(&self, func: &Function) -> Result<MachFunction, BackendError> {
        match self.select_scalar(func) {
            Ok(mach) => Ok(mach),
            Err(_) => Ok(MachFunction::fallback(func)),
        }
    }
}

impl RirSelector {
    fn select_scalar(&self, func: &Function) -> Result<MachFunction, BackendError> {
        if disabled("RAKIT_NO_CUSTOM_SCALAR") {
            return Err(unsupported("scalar disabled"));
        }
        if func.ret.len() > 1 {
            return Err(unsupported("multiple return values"));
        }
        if func.blocks.iter().skip(1).any(|block| !block.params.is_empty()) {
            return Err(unsupported("non-entry block parameters"));
        }

        let ret = func.ret.first().copied();
        let mut mach = MachFunction::new(ret);
        mach.externals = func.externals.clone();
        mach.globals = func.globals.clone();
        mach.stack_slots = func.stack_slots.clone();
        mach.dynamic_stack_slots = func.dynamic_stack_slots.clone();
        mach.dynamic_types = func.dynamic_types.clone();
        let mut vmap: HashMap<Value, VReg> = HashMap::new();
        let mut icmp_defs: HashMap<Value, (IntCc, Operand, Operand)> = HashMap::new();

        for _ in &func.blocks {
            mach.add_block();
        }

        for param in &func.block(func.entry).params {
            let ty = func.ty(*param);
            check_scalar(ty)?;
            let vreg = mach.add_vreg(ty);
            mach.params.push(vreg);
            vmap.insert(*param, vreg);
        }

        for (bi, block) in func.blocks.iter().enumerate() {
            for inst in &block.insts {
                match inst {
                    Inst::Nop(..) => {}
                    Inst::F32const(..) | Inst::F64const(..) => {
                        return Err(unsupported("float constants"));
                    }
                    Inst::Iconst(r, ty, value) => {
                        check_scalar(*ty)?;
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        mach.blocks[bi].insts.push(MachInst::Mov {
                            dst: Operand::VReg(dst),
                            src: Operand::Imm(*value),
                        });
                    }
                    Inst::Copy(r, ty, x) => {
                        check_scalar(*ty)?;
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        let src = operand(&vmap, *x)?;
                        mach.blocks[bi].insts.push(MachInst::Mov { dst: Operand::VReg(dst), src });
                    }
                    Inst::Unary(..) => return Err(unsupported("type conversion")),
                    Inst::Bin(r, op, ty, x, y) => {
                        check_scalar(*ty)?;
                        let op = match op {
                            BinOp::Add => MachBinOp::Add,
                            BinOp::Sub => MachBinOp::Sub,
                            BinOp::Mul => MachBinOp::Mul,
                            BinOp::And => MachBinOp::And,
                            BinOp::Or => MachBinOp::Or,
                            BinOp::Xor => MachBinOp::Xor,
                            BinOp::Shl => MachBinOp::Shl,
                            BinOp::Shr => MachBinOp::Shr,
                            BinOp::Ushr => MachBinOp::Ushr,
                            other => {
                                return Err(unsupported(&format!("binop {other:?}")));
                            }
                        };
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        let lhs = operand(&vmap, *x)?;
                        let rhs = operand(&vmap, *y)?;
                        mach.blocks[bi].insts.push(MachInst::Bin {
                            op,
                            dst: Operand::VReg(dst),
                            lhs,
                            rhs,
                        });
                    }
                    Inst::Icmp(r, cc, ty, x, y) => {
                        check_scalar(*ty)?;
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        let lhs = operand(&vmap, *x)?;
                        let rhs = operand(&vmap, *y)?;
                        mach.blocks[bi].insts.push(MachInst::Cmp { lhs, rhs });
                        mach.blocks[bi]
                            .insts
                            .push(MachInst::SetCC { dst: Operand::VReg(dst), cc: *cc });
                        icmp_defs.insert(*r, (*cc, lhs, rhs));
                    }
                    Inst::Brif(cond, then, then_args, otherwise, else_args) => {
                        if !then_args.is_empty() || !else_args.is_empty() {
                            return Err(unsupported("branch block arguments"));
                        }
                        let then = then.0 as usize;
                        let otherwise = otherwise.0 as usize;
                        if then == otherwise {
                            mach.blocks[bi].insts.push(MachInst::Jmp { target: then });
                            continue;
                        }
                        if let Some((cc, lhs, rhs)) = icmp_defs.get(cond) {
                            mach.blocks[bi].insts.push(MachInst::Cmp { lhs: *lhs, rhs: *rhs });
                            mach.blocks[bi].insts.push(MachInst::Jcc { cc: *cc, target: then });
                        } else {
                            let cond = operand(&vmap, *cond)?;
                            mach.blocks[bi]
                                .insts
                                .push(MachInst::Cmp { lhs: cond, rhs: Operand::Imm(0) });
                            mach.blocks[bi]
                                .insts
                                .push(MachInst::Jcc { cc: IntCc::Ne, target: then });
                        }
                        mach.blocks[bi].insts.push(MachInst::Jmp { target: otherwise });
                    }
                    Inst::Jump(target, args) => {
                        if !args.is_empty() {
                            return Err(unsupported("jump block arguments"));
                        }
                        mach.blocks[bi].insts.push(MachInst::Jmp { target: target.0 as usize });
                    }
                    Inst::Return(args) => {
                        if args.len() > 1 {
                            return Err(unsupported("multiple return values"));
                        }
                        let value = match args.first() {
                            Some(v) => Some(operand(&vmap, *v)?),
                            None => None,
                        };
                        mach.blocks[bi].insts.push(MachInst::Ret { value });
                    }
                    Inst::Select(r, ty, cond, then, otherwise) => {
                        check_scalar(*ty)?;
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        let cond = operand(&vmap, *cond)?;
                        let then = operand(&vmap, *then)?;
                        let otherwise = operand(&vmap, *otherwise)?;
                        mach.blocks[bi].insts.push(MachInst::Select {
                            dst: Operand::VReg(dst),
                            cond,
                            then,
                            otherwise,
                        });
                    }
                    Inst::Call(results, target, args) => {
                        if disabled("RAKIT_NO_CUSTOM_CALL") {
                            return Err(unsupported("calls disabled"));
                        }
                        let results = results
                            .iter()
                            .map(|r| result_vreg(&mut mach, &mut vmap, func, *r))
                            .collect::<Vec<_>>();
                        let args = args
                            .iter()
                            .map(|a| operand(&vmap, *a))
                            .collect::<Result<Vec<_>, _>>()?;
                        mach.blocks[bi].insts.push(MachInst::Call {
                            results: results.into_iter().map(Operand::VReg).collect(),
                            target: *target,
                            args,
                        });
                    }
                    Inst::CallIndirect {
                        results,
                        params,
                        rets,
                        call_conv,
                        callee,
                        args,
                    } => {
                        if disabled("RAKIT_NO_CUSTOM_CALL") {
                            return Err(unsupported("calls disabled"));
                        }
                        let results = results
                            .iter()
                            .map(|r| result_vreg(&mut mach, &mut vmap, func, *r))
                            .collect::<Vec<_>>();
                        let callee = operand(&vmap, *callee)?;
                        let args = args
                            .iter()
                            .map(|a| operand(&vmap, *a))
                            .collect::<Result<Vec<_>, _>>()?;
                        mach.blocks[bi].insts.push(MachInst::CallIndirect {
                            results: results.into_iter().map(Operand::VReg).collect(),
                            params: params.clone(),
                            rets: rets.clone(),
                            call_conv: *call_conv,
                            callee,
                            args,
                        });
                    }
                    Inst::TryCall { .. } => return Err(unsupported("try calls")),
                    Inst::TryCallIndirect { .. } => {
                        return Err(unsupported("indirect try calls"))
                    }
                    Inst::Fcmp(..) => return Err(unsupported("float compare")),
                    Inst::StackAddr(r, slot, offset) => {
                        if disabled("RAKIT_NO_CUSTOM_MEM") {
                            return Err(unsupported("memory disabled"));
                        }
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        mach.blocks[bi].insts.push(MachInst::StackAddr {
                            dst: Operand::VReg(dst),
                            slot: *slot,
                            offset: *offset,
                        });
                    }
                    Inst::Load(r, ty, addr, offset) => {
                        if disabled("RAKIT_NO_CUSTOM_MEM") {
                            return Err(unsupported("memory disabled"));
                        }
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        let addr = operand(&vmap, *addr)?;
                        mach.blocks[bi].insts.push(MachInst::Load {
                            dst: Operand::VReg(dst),
                            addr,
                            offset: *offset,
                            ty: *ty,
                        });
                    }
                    Inst::Store(ty, addr, offset, value) => {
                        if disabled("RAKIT_NO_CUSTOM_MEM") {
                            return Err(unsupported("memory disabled"));
                        }
                        let addr = operand(&vmap, *addr)?;
                        let value = operand(&vmap, *value)?;
                        mach.blocks[bi].insts.push(MachInst::Store {
                            addr,
                            offset: *offset,
                            value,
                            ty: *ty,
                        });
                    }
                    Inst::GlobalValue(r, index) => {
                        if disabled("RAKIT_NO_CUSTOM_GLOBAL") {
                            return Err(unsupported("globals disabled"));
                        }
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        mach.blocks[bi].insts.push(MachInst::GlobalValue {
                            dst: Operand::VReg(dst),
                            index: *index,
                        });
                    }
                    Inst::FuncAddr(r, target) => {
                        if disabled("RAKIT_NO_CUSTOM_GLOBAL") {
                            return Err(unsupported("globals disabled"));
                        }
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        mach.blocks[bi].insts.push(MachInst::FuncAddr {
                            dst: Operand::VReg(dst),
                            target: *target,
                        });
                    }
                    Inst::DynamicStackAddr(r, index) => {
                        if disabled("RAKIT_NO_CUSTOM_GLOBAL") {
                            return Err(unsupported("globals disabled"));
                        }
                        let dst = result_vreg(&mut mach, &mut vmap, func, *r);
                        mach.blocks[bi].insts.push(MachInst::DynamicStackAddr {
                            dst: Operand::VReg(dst),
                            index: *index,
                        });
                    }
                    Inst::Trap(code) => {
                        if disabled("RAKIT_NO_CUSTOM_TRAP") {
                            return Err(unsupported("trap disabled"));
                        }
                        mach.blocks[bi].insts.push(MachInst::Trap(*code));
                    }
                }
            }
            if !matches!(
                block.insts.last(),
                Some(Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..) | Inst::Trap(..))
            ) {
                return Err(unsupported("block without terminator"));
            }
        }

        Ok(mach)
    }
}

fn disabled(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn unsupported(what: &str) -> BackendError {
    BackendError::Unsupported(what.to_string())
}

fn check_scalar(ty: Type) -> Result<(), BackendError> {
    match ty {
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr | Type::Bool => Ok(()),
        Type::I128 => Err(unsupported("I128 arithmetic")),
        Type::F32 | Type::F64 => Err(unsupported("float arithmetic")),
    }
}

fn result_vreg(
    mach: &mut MachFunction,
    vmap: &mut HashMap<Value, VReg>,
    func: &Function,
    value: Value,
) -> VReg {
    let vreg = mach.add_vreg(func.ty(value));
    vmap.insert(value, vreg);
    vreg
}

fn operand(vmap: &HashMap<Value, VReg>, value: Value) -> Result<Operand, BackendError> {
    vmap.get(&value)
        .copied()
        .map(Operand::VReg)
        .ok_or_else(|| unsupported(&format!("value {value:?} has no machine register")))
}
