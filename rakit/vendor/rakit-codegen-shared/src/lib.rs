//! This library contains code that is common to both the `rakit-codegen` and
//! `rakit-codegen-meta` libraries.

#![deny(missing_docs)]
#![no_std]

pub mod constant_hash;
pub mod constants;

/// Version number of this crate.
pub const VERSION: &str = core::env!("CARGO_PKG_VERSION");
