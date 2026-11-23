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
//! A later step will add the codegen IR bridge (import/export) and move existing
//! passes onto this IR.

pub(crate) mod cfg_util;

pub mod backend;
pub mod builder;
pub mod div_test;
pub mod gvn;
pub mod if_convert;
pub mod inliner;
pub mod int_sem;
pub mod interp;
pub mod ir;
pub mod licm;
pub mod mem_forward;
pub mod opt;
pub mod rec_loop;
pub mod simplify;
pub mod sincos;
pub mod sroa;
pub mod unroll;
pub mod verify;

pub use builder::Builder;
pub use interp::{Val, run, run_with, run_with_indirect};
pub use ir::*;
pub use opt::optimize;
pub use verify::verify;
