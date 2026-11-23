//! Reference interpreter for allocated machine IR.
//!
//! This is the executable oracle used by tests: selection and register
//! allocation are only accepted when this interpreter agrees with the RIR
//! interpreter.

use crate::backend::mach::{MachBinOp, MachFunction, MachInst, Operand};
use crate::ir::IntCc;

pub fn run(func: &MachFunction, args: &[i64]) -> Result<Option<i64>, String> {
    if args.len() != func.param_locs.len() {
        return Err(format!("expected {} arguments, got {}", func.param_locs.len(), args.len()));
    }

    let mut regs = [0i64; 16];
    let mut spills = vec![0i64; func.spill_slots as usize];
    for (loc, value) in func.param_locs.iter().zip(args) {
        write(&mut regs, &mut spills, *loc, *value)?;
    }

    let mut block = 0usize;
    let mut pc = 0usize;
    let mut last_cmp: Option<(i64, i64, IntCc)> = None;
    let mut steps = 0usize;

    loop {
        steps += 1;
        if steps > 1_000_000 {
            return Err("machine interpreter step limit exceeded".to_string());
        }
        let inst = func
            .blocks
            .get(block)
            .and_then(|b| b.insts.get(pc))
            .ok_or_else(|| format!("pc out of range in block {block}"))?
            .clone();
        pc += 1;

        match inst {
            MachInst::Mov { dst, src } => {
                let value = read(&regs, &spills, src)?;
                write(&mut regs, &mut spills, dst, value)?;
            }
            MachInst::Bin { op, dst, lhs, rhs } => {
                let lhs = read(&regs, &spills, lhs)?;
                let rhs = read(&regs, &spills, rhs)?;
                let value = match op {
                    MachBinOp::Add => lhs.wrapping_add(rhs),
                    MachBinOp::Sub => lhs.wrapping_sub(rhs),
                    MachBinOp::Mul => lhs.wrapping_mul(rhs),
                    MachBinOp::And => lhs & rhs,
                    MachBinOp::Or => lhs | rhs,
                    MachBinOp::Xor => lhs ^ rhs,
                    MachBinOp::Shl => lhs.wrapping_shl(rhs as u32),
                    MachBinOp::Shr => lhs.wrapping_shr(rhs as u32),
                    MachBinOp::Ushr => (lhs as u64).wrapping_shr(rhs as u32) as i64,
                };
                write(&mut regs, &mut spills, dst, value)?;
            }
            MachInst::Cmp { lhs, rhs } => {
                let lhs = read(&regs, &spills, lhs)?;
                let rhs = read(&regs, &spills, rhs)?;
                last_cmp = Some((lhs, rhs, IntCc::Eq));
            }
            MachInst::Select { dst, cond, then, otherwise } => {
                let cond = read(&regs, &spills, cond)?;
                let value = if cond != 0 {
                    read(&regs, &spills, then)?
                } else {
                    read(&regs, &spills, otherwise)?
                };
                write(&mut regs, &mut spills, dst, value)?;
            }
            MachInst::SetCC { dst, cc } => {
                let (lhs, rhs, _) = last_cmp.ok_or("SetCC without Cmp")?;
                write(&mut regs, &mut spills, dst, eval_cc(cc, lhs, rhs) as i64)?;
            }
            MachInst::Jcc { cc, target } => {
                let (lhs, rhs, _) = last_cmp.ok_or("Jcc without Cmp")?;
                if eval_cc(cc, lhs, rhs) {
                    block = target;
                    pc = 0;
                }
            }
            MachInst::Jmp { target } => {
                block = target;
                pc = 0;
            }
            MachInst::Ret { value } => {
                return match value {
                    Some(value) => Ok(Some(read(&regs, &spills, value)?)),
                    None => Ok(None),
                };
            }
            other => return Err(format!("machine interpreter does not support {other:?}")),
        }
    }
}

fn read(regs: &[i64; 16], spills: &[i64], operand: Operand) -> Result<i64, String> {
    match operand {
        Operand::Imm(value) => Ok(value),
        Operand::PReg(reg) => regs
            .get(reg.0 as usize)
            .copied()
            .ok_or_else(|| format!("physical register {} out of range", reg.0)),
        Operand::Spill(slot) => spills
            .get(slot as usize)
            .copied()
            .ok_or_else(|| format!("spill slot {slot} out of range")),
        Operand::VReg(v) => Err(format!("unallocated virtual register {v}")),
    }
}

fn write(
    regs: &mut [i64; 16],
    spills: &mut [i64],
    operand: Operand,
    value: i64,
) -> Result<(), String> {
    match operand {
        Operand::PReg(reg) => {
            let slot = regs
                .get_mut(reg.0 as usize)
                .ok_or_else(|| format!("physical register {} out of range", reg.0))?;
            *slot = value;
            Ok(())
        }
        Operand::Spill(slot) => {
            let slot = spills
                .get_mut(slot as usize)
                .ok_or_else(|| format!("spill slot {slot} out of range"))?;
            *slot = value;
            Ok(())
        }
        Operand::Imm(_) | Operand::VReg(_) => Err(format!("cannot write to {operand}")),
    }
}

fn eval_cc(cc: IntCc, lhs: i64, rhs: i64) -> bool {
    match cc {
        IntCc::Eq => lhs == rhs,
        IntCc::Ne => lhs != rhs,
        IntCc::Slt => lhs < rhs,
        IntCc::Sle => lhs <= rhs,
        IntCc::Sgt => lhs > rhs,
        IntCc::Sge => lhs >= rhs,
        IntCc::Ult => (lhs as u64) < (rhs as u64),
        IntCc::Ule => (lhs as u64) <= (rhs as u64),
        IntCc::Ugt => (lhs as u64) > (rhs as u64),
        IntCc::Uge => (lhs as u64) >= (rhs as u64),
    }
}
