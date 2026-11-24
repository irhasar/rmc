//! Rakit's codegen IR-level optimization layer.
//!
//! This crate owns the optimization pipeline that runs on a function before it
//! is handed to the machine-code emitter: a pass manager, a cache of shared
//! function analyses, and the first-party passes themselves. Keeping it out of
//! `rustc_codegen_rakit` lets the pipeline be built, tested and reused without
//! the rustc-private dependencies the backend needs.

pub mod analyses;
pub mod pass;
pub mod passes;
pub mod passtime;
pub mod tunables;

pub use analyses::Analyses;
pub use pass::{Pass, PassCtx, PassManager};
pub use passes::{
    DivTestPass, IfConvertPass, RecLoopPass, RotatePass, SincosPass, StoreForwardPass,
    UnrollPass, VectorizePass,
};
