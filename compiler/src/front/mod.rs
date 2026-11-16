//! rmc's own front end for the Rust subset in `docs/language.md`: lexing,
//! parsing (with the desugarings that keep later stages small), and semantic
//! analysis (types, layout, slots, and the soundness checks).
//!
//! The front end exists for compile speed: it turns a program into an
//! executable in a few milliseconds without running rustc. A program outside
//! the subset is rejected with a message; the `--rustc-frontend` route
//! (`crate::mir`) covers more of the language by letting rustc do this work.

pub mod ast;
pub mod consts;
pub mod lexer;
pub mod parser;
pub mod sema;
