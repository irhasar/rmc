//! Small machine IR used by the Phase 3 backend.
//!
//! Values start as virtual registers. The register allocator rewrites every
//! `VReg` operand to a physical register or a spill slot.

use std::fmt;

use crate::ir::{BinOp, Block, BlockData, Function, Inst, IntCc, Type, Value};

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

/// Lower a virtual-register machine function back to RIR.  This is the
/// production emitter for the scalar subset selected by [`RirSelector`]:
/// callers run `LinearScan` for validation and then emit this RIR, so the
/// machine backend is on the codegen path rather than only in tests.
pub fn to_rir(mach: &MachFunction) -> Result<Function, String> {
    let entry_params = mach
        .params
        .iter()
        .map(|v| mach.vregs.get(v.0 as usize).copied().ok_or("bad parameter vreg"))
        .collect::<Result<Vec<_>, _>>()?;
    let mut func = Function::new(mach.ret.into_iter().collect(), entry_params);
    let entry = func.entry;
    let mut vmap: std::collections::HashMap<VReg, Value> = std::collections::HashMap::new();
    for (v, param) in mach.params.iter().zip(func.block(entry).params.clone()) {
        vmap.insert(*v, param);
    }
    for (i, ty) in mach.vregs.iter().enumerate() {
        let vreg = VReg(i as u32);
        if vmap.contains_key(&vreg) {
            continue;
        }
        let value = Value(func.values.len() as u32);
        func.values.push(*ty);
        vmap.insert(vreg, value);
    }
    for _ in 1..mach.blocks.len() {
        let b = Block(func.blocks.len() as u32);
        func.blocks.push(BlockData { params: Vec::new(), insts: Vec::new() });
        debug_assert_eq!(b.0 as usize, func.blocks.len() - 1);
    }

    fn value_of(
        func: &mut Function,
        block: Block,
        vmap: &std::collections::HashMap<VReg, Value>,
        operand: Operand,
    ) -> Result<Value, String> {
        match operand {
            Operand::VReg(v) => vmap.get(&v).copied().ok_or_else(|| format!("unknown {v:?}")),
            Operand::Imm(imm) => {
                let value = Value(func.values.len() as u32);
                func.values.push(Type::I64);
                func.blocks[block.0 as usize].insts.push(Inst::Iconst(value, Type::I64, imm));
                Ok(value)
            }
            Operand::PReg(_) | Operand::Spill(_) => {
                Err("physical register/spill in pre-allocation machine IR".to_string())
            }
        }
    }

    for (bi, block) in mach.blocks.iter().enumerate() {
        let rb = Block(bi as u32);
        let mut pending_cmp: Option<(Operand, Operand)> = None;
        let mut skip_next_jmp = false;
        for (ii, inst) in block.insts.iter().enumerate() {
            if skip_next_jmp {
                skip_next_jmp = false;
                continue;
            }
            match inst {
                MachInst::Cmp { lhs, rhs } => pending_cmp = Some((*lhs, *rhs)),
                MachInst::Mov { dst, src } => {
                    let d = value_of(&mut func, rb, &vmap, *dst)?;
                    let ty = func.ty(d);
                    match *src {
                        Operand::Imm(imm) => {
                            func.blocks[bi].insts.push(Inst::Iconst(d, ty, imm));
                        }
                        Operand::VReg(v) => {
                            let x = *vmap.get(&v).ok_or("unknown move source")?;
                            func.blocks[bi].insts.push(Inst::Copy(d, ty, x));
                        }
                        Operand::PReg(_) | Operand::Spill(_) => {
                            return Err("physical register/spill in pre-allocation machine IR".to_string())
                        }
                    }
                }
                MachInst::Bin { op, dst, lhs, rhs } => {
                    let d = value_of(&mut func, rb, &vmap, *dst)?;
                    let ty = func.ty(d);
                    let x = value_of(&mut func, rb, &vmap, *lhs)?;
                    let y = value_of(&mut func, rb, &vmap, *rhs)?;
                    let op = match op {
                        MachBinOp::Add => BinOp::Add,
                        MachBinOp::Sub => BinOp::Sub,
                        MachBinOp::Mul => BinOp::Mul,
                        MachBinOp::And => BinOp::And,
                        MachBinOp::Or => BinOp::Or,
                        MachBinOp::Xor => BinOp::Xor,
                        MachBinOp::Shl => BinOp::Shl,
                        MachBinOp::Shr => BinOp::Shr,
                        MachBinOp::Ushr => BinOp::Ushr,
                    };
                    func.blocks[bi].insts.push(Inst::Bin(d, op, ty, x, y));
                }
                MachInst::SetCC { dst, cc } => {
                    let (lhs, rhs) = pending_cmp.take().ok_or("SetCC without Cmp")?;
                    let d = value_of(&mut func, rb, &vmap, *dst)?;
                    let x = value_of(&mut func, rb, &vmap, lhs)?;
                    let y = value_of(&mut func, rb, &vmap, rhs)?;
                    let ty = func.ty(x);
                    func.blocks[bi].insts.push(Inst::Icmp(d, *cc, ty, x, y));
                }
                MachInst::Select { dst, cond, then, otherwise } => {
                    let d = value_of(&mut func, rb, &vmap, *dst)?;
                    let ty = func.ty(d);
                    let c = value_of(&mut func, rb, &vmap, *cond)?;
                    let x = value_of(&mut func, rb, &vmap, *then)?;
                    let y = value_of(&mut func, rb, &vmap, *otherwise)?;
                    func.blocks[bi].insts.push(Inst::Select(d, ty, c, x, y));
                }
                MachInst::Jcc { cc, target } => {
                    let (lhs, rhs) = pending_cmp.take().ok_or("Jcc without Cmp")?;
                    let cond = Value(func.values.len() as u32);
                    func.values.push(Type::Bool);
                    let x = value_of(&mut func, rb, &vmap, lhs)?;
                    let y = value_of(&mut func, rb, &vmap, rhs)?;
                    let ty = func.ty(x);
                    func.blocks[bi].insts.push(Inst::Icmp(cond, *cc, ty, x, y));
                    let else_target = match block.insts.get(ii + 1) {
                        Some(MachInst::Jmp { target }) => {
                            skip_next_jmp = true;
                            *target
                        }
                        _ => bi + 1,
                    };
                    if else_target >= mach.blocks.len() {
                        return Err("Jcc has no else target".to_string());
                    }
                    func.blocks[bi].insts.push(Inst::Brif(
                        cond,
                        Block(*target as u32),
                        Vec::new(),
                        Block(else_target as u32),
                        Vec::new(),
                    ));
                }
                MachInst::Jmp { target } => {
                    func.blocks[bi].insts.push(Inst::Jump(Block(*target as u32), Vec::new()));
                }
                MachInst::Ret { value } => {
                    let value = match value {
                        Some(v) => Some(value_of(&mut func, rb, &vmap, *v)?),
                        None => None,
                    };
                    func.blocks[bi]
                        .insts
                        .push(Inst::Return(value.into_iter().collect()));
                }
            }
        }
    }
    Ok(func)
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
