//! `rakit-ir`: a small, self-contained SSA mid-level IR for Rakit.
//!
//! This is the foundation of Rakit's own middle-end. It is deliberately
//! independent of the machine-code layer so that passes can be written, verified
//! and tested without building a full backend. The first milestone covers a
//! scalar subset (integer/float arithmetic, comparisons, select, branches and
//! returns) with:
//!
//! - [`ir`]: the IR itself (types, values, blocks, instructions, function);
//! - [`builder`]: an ergonomic function builder;
//! - [`verify`]: an SSA verifier (single definition, dominance, types);
//! - [`opt`]: a first optimizer (constant folding, branch folding, dead code
//!   elimination);
//! - [`interp`]: a reference interpreter used by the tests.
//!
//! A later step will add the CLIF bridge (import/export) and move existing
//! passes onto this IR.

pub mod builder;
pub mod div_test;
pub mod interp;
pub mod ir;
pub mod opt;
pub mod verify;

pub use builder::Builder;
pub use interp::{Val, run};
pub use ir::*;
pub use opt::optimize;
pub use verify::verify;
