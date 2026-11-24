//! Optimisation passes over the AST, run after `sema` and before the x86
//! code generator, in this order: `inline` (small call-free functions),
//! `tco` (self-recursion to a loop), `licm` (loop-invariant code motion) and
//! `dce` (functions `main` cannot reach). Each has a `--no-*` switch so a bug
//! can be isolated to one pass.

pub mod dce;
pub mod inline;
pub mod licm;
pub mod tco;
