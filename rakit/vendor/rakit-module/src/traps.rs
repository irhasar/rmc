//! Defines `TrapSite`.

use rakit_codegen::{binemit, ir};

/// Record of the arguments rakit passes to `TrapSink::trap`.
#[derive(Clone, Debug)]
pub struct TrapSite {
    /// Offset into function.
    pub offset: binemit::CodeOffset,
    /// Source location given to rakit.
    pub srcloc: ir::SourceLoc,
    /// Trap code, as determined by rakit.
    pub code: ir::TrapCode,
}
