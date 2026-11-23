//! Phase 3 backend boundary: instruction selection and register allocation
//! are behind traits so a different selector or allocator can be swapped in.

use std::fmt;

use crate::ir::Function;

pub mod exec;
pub mod isel;
pub mod mach;
pub mod regalloc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    Unsupported(String),
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for BackendError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AllocStats {
    pub vregs: usize,
    pub registers: usize,
    pub spills: usize,
}

/// Lower a verified RIR function to machine IR. Implementations must reject
/// anything they cannot represent instead of emitting wrong code.
pub trait InstructionSelector {
    fn select(&self, func: &Function) -> Result<mach::MachFunction, BackendError>;
}

/// Assign every virtual register to a physical register or a spill slot.
pub trait RegisterAllocator {
    fn allocate(&self, func: &mut mach::MachFunction) -> Result<AllocStats, BackendError>;
}

pub use exec::run as run_machine;
pub use isel::RirSelector;
pub use mach::{MachFunction, Operand, PReg, VReg};
pub use regalloc::LinearScan;
