//! The x86-64 back end shared by both routes: the AST code generator
//! (`codegen`), the freestanding runtime (`runtime`, raw Linux syscalls, no
//! libc) and the in-process assembler and static ELF64 linker (`elf`). The MIR
//! route's lowering lives in `crate::mir::codegen` and feeds the same
//! assembler and runtime.

pub mod codegen;
pub mod elf;
pub mod runtime;
