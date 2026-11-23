//! Small machine IR used by the Phase 3 backend.
//!
//! Values start as virtual registers. The register allocator rewrites every
//! `VReg` operand to a physical register or a spill slot.

use std::fmt;

use crate::ir::{IntCc, Type};

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct VReg(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct PReg(pub u8);

pub const RAX: PReg = PReg(0);
pub const RCX: PReg = PReg(1);
pub const RDX: PReg = PReg(2);
pub const RSI: PReg = PReg(6);
pub const RDI: PReg = PReg(7);
pub const R8: PReg = PReg(8);
pub const R9: PReg = PReg(9);
pub const R10: PReg = PReg(10);
pub const R11: PReg = PReg(11);
pub const R12: PReg = PReg(12);

/// Registers the allocator may assign. RSP/RBP are deliberately excluded.
pub const ALLOCATABLE: [PReg; 10] = [RAX, RCX, RDX, RSI, RDI, R8, R9, R10, R11, R12];

pub fn preg_name(reg: PReg) -> &'static str {
    match reg {
        RAX => "%rax",
        RCX => "%rcx",
        RDX => "%rdx",
        RSI => "%rsi",
        RDI => "%rdi",
        R8 => "%r8",
        R9 => "%r9",
        R10 => "%r10",
        R11 => "%r11",
        R12 => "%r12",
        _ => "%?",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand {
    Imm(i64),
    VReg(VReg),
    PReg(PReg),
    Spill(u32),
}

impl Operand {
    pub fn vreg(self) -> Option<VReg> {
        match self {
            Operand::VReg(v) => Some(v),
            _ => None,
        }
    }

    pub fn map_vreg(self, f: &mut impl FnMut(VReg) -> Operand) -> Operand {
        match self {
            Operand::VReg(v) => f(v),
            other => other,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MachBinOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Ushr,
}

#[derive(Clone, Debug)]
pub enum MachInst {
    Mov { dst: Operand, src: Operand },
    Bin { op: MachBinOp, dst: Operand, lhs: Operand, rhs: Operand },
    Cmp { lhs: Operand, rhs: Operand },
    SetCC { dst: Operand, cc: IntCc },
    Select { dst: Operand, cond: Operand, then: Operand, otherwise: Operand },
    Jcc { cc: IntCc, target: usize },
    Jmp { target: usize },
    Ret { value: Option<Operand> },
}

impl MachInst {
    pub fn defs(&self) -> Vec<VReg> {
        let mut out = Vec::new();
        match self {
            MachInst::Mov { dst, .. } | MachInst::SetCC { dst, .. } => {
                if let Some(v) = dst.vreg() {
                    out.push(v);
                }
            }
            MachInst::Bin { dst, .. } => {
                if let Some(v) = dst.vreg() {
                    out.push(v);
                }
            }
            MachInst::Cmp { .. }
            | MachInst::Select { .. }
            | MachInst::Jcc { .. }
            | MachInst::Jmp { .. }
            | MachInst::Ret { .. } => {}
        }
        out
    }

    pub fn uses(&self) -> Vec<VReg> {
        let mut out = Vec::new();
        match self {
            MachInst::Mov { src, .. } => {
                if let Some(v) = src.vreg() {
                    out.push(v);
                }
            }
            MachInst::Bin { lhs, rhs, .. } => {
                if let Some(v) = lhs.vreg() {
                    out.push(v);
                }
                if let Some(v) = rhs.vreg() {
                    out.push(v);
                }
            }
            MachInst::Cmp { lhs, rhs } => {
                if let Some(v) = lhs.vreg() {
                    out.push(v);
                }
                if let Some(v) = rhs.vreg() {
                    out.push(v);
                }
            }
            MachInst::Select { cond, then, otherwise, .. } => {
                if let Some(v) = cond.vreg() {
                    out.push(v);
                }
                if let Some(v) = then.vreg() {
                    out.push(v);
                }
                if let Some(v) = otherwise.vreg() {
                    out.push(v);
                }
            }
            MachInst::Ret { value: Some(v) } => {
                if let Some(v) = v.vreg() {
                    out.push(v);
                }
            }
            MachInst::SetCC { .. }
            | MachInst::Jcc { .. }
            | MachInst::Jmp { .. }
            | MachInst::Ret { value: None } => {}
        }
        out
    }

    pub fn map_vregs(&mut self, f: &mut impl FnMut(VReg) -> Operand) {
        match self {
            MachInst::Mov { dst, src } => {
                *dst = dst.map_vreg(f);
                *src = src.map_vreg(f);
            }
            MachInst::Bin { dst, lhs, rhs, .. } => {
                *dst = dst.map_vreg(f);
                *lhs = lhs.map_vreg(f);
                *rhs = rhs.map_vreg(f);
            }
            MachInst::Cmp { lhs, rhs } => {
                *lhs = lhs.map_vreg(f);
                *rhs = rhs.map_vreg(f);
            }
            MachInst::Select { dst, cond, then, otherwise } => {
                *dst = dst.map_vreg(f);
                *cond = cond.map_vreg(f);
                *then = then.map_vreg(f);
                *otherwise = otherwise.map_vreg(f);
            }
            MachInst::SetCC { dst, .. } => {
                *dst = dst.map_vreg(f);
            }
            MachInst::Ret { value } => {
                if let Some(v) = value {
                    *v = v.map_vreg(f);
                }
            }
            MachInst::Jcc { .. } | MachInst::Jmp { .. } => {}
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MachBlock {
    pub insts: Vec<MachInst>,
}

#[derive(Clone, Debug)]
pub struct MachFunction {
    pub blocks: Vec<MachBlock>,
    /// Entry parameters, still virtual until register allocation.
    pub params: Vec<VReg>,
    /// Allocated location for each entry parameter.
    pub param_locs: Vec<Operand>,
    pub ret: Option<Type>,
    pub vregs: Vec<Type>,
    pub spill_slots: u32,
}

impl MachFunction {
    pub fn new(ret: Option<Type>) -> Self {
        Self {
            blocks: Vec::new(),
            params: Vec::new(),
            param_locs: Vec::new(),
            ret,
            vregs: Vec::new(),
            spill_slots: 0,
        }
    }

    pub fn add_vreg(&mut self, ty: Type) -> VReg {
        let v = VReg(self.vregs.len() as u32);
        self.vregs.push(ty);
        v
    }

    pub fn add_block(&mut self) -> usize {
        let b = self.blocks.len();
        self.blocks.push(MachBlock::default());
        b
    }

    pub fn successors(&self, block: usize) -> Vec<usize> {
        let mut out = Vec::new();
        for inst in &self.blocks[block].insts {
            if let MachInst::Jcc { target, .. } = inst {
                out.push(*target);
            }
        }
        if let Some(MachInst::Jmp { target }) = self.blocks[block].insts.last() {
            out.push(*target);
        }
        out
    }
}

impl fmt::Display for VReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Imm(v) => write!(f, "{v}"),
            Operand::VReg(v) => write!(f, "{v}"),
            Operand::PReg(r) => write!(f, "{}", preg_name(*r)),
            Operand::Spill(s) => write!(f, "spill{s}"),
        }
    }
}
