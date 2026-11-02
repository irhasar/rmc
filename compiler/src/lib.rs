//! nrc: an ahead-of-time compiler for a small, statically-typed subset of Rust.
//!
//! The pipeline is lex -> parse -> semantic analysis -> x86-64 assembly. This
//! library exposes it as a single call, [`compile_to_asm`], so nrc can be
//! embedded in another program instead of only run as a command-line tool. The
//! `nrc` binary in `src/main.rs` drives this library and then invokes `as` and
//! `ld`; those two are the only external dependencies, and neither is needed if
//! you only want the assembly text.
//!
//! ```
//! let asm = nrc::compile_to_asm("fn main() -> i64 { return 0; }", &Default::default())
//!     .expect("compiles");
//! assert!(asm.contains("main:"));
//! ```

pub mod ast;
pub mod codegen;
pub mod lexer;
pub mod parser;
pub mod runtime;
pub mod sema;

/// Options that change what the compiler emits.
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Insert a trap after every `+`, `-` and `*` so that signed overflow exits
    /// with status 101 instead of wrapping. Off by default because it costs a
    /// branch per arithmetic operation.
    pub overflow_checks: bool,
}

/// Compile `src` (a `.nrs` program) to x86-64 assembly in GNU `as` syntax.
///
/// The returned string is a complete translation unit: the compiled program
/// followed by the freestanding runtime. Feed it to `as` and `ld` to obtain an
/// executable. Every error is a human-readable string with a byte offset into
/// `src`.
pub fn compile_to_asm(src: &str, opts: &CompileOptions) -> Result<String, String> {
    let toks = lexer::lex(src)?;
    let mut prog = parser::Parser::new(toks).parse_program()?;
    sema::analyze(&mut prog)?;

    let mut cg = codegen::Codegen::new();
    if opts.overflow_checks {
        cg = cg.overflow_checks();
    }
    let user_asm = cg.compile(&prog);

    let mut asm = String::with_capacity(user_asm.len() + runtime::RUNTIME_ASM.len() + 64);
    asm.push_str(&user_asm);
    asm.push_str(runtime::RUNTIME_ASM);
    Ok(asm)
}
