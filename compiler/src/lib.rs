//! rmc: an ahead-of-time compiler for a small, statically-typed subset of Rust.
//!
//! The pipeline is lex -> parse -> semantic analysis -> inline -> x86-64
//! assembly. This library exposes it as a single call, [`compile_to_asm`], so
//! rmc can be embedded in another program instead of only run as a
//! command-line tool. The `rmc` binary in `src/main.rs` drives this library and
//! then invokes `as` and `ld`; those two are the only external dependencies, and
//! neither is needed if you only want the assembly text.
//!
//! ```
//! let asm = rmc::compile_to_asm("fn main() -> i64 { return 0; }", &Default::default())
//!     .expect("compiles");
//! assert!(asm.contains("main:"));
//! ```

pub mod ast;
pub mod codegen;
pub mod inline;
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
    /// Insert a bounds check before every array index so that an out-of-range
    /// index exits with status 101 instead of reading or writing adjacent stack
    /// memory. Off by default because it costs a compare and branch per index.
    pub bounds_checks: bool,
    /// Skip the inlining pass. Inlining is on by default: it is what makes a
    /// call to a small accessor as cheap as the arithmetic it wraps.
    pub no_inline: bool,
}

/// Compile `src` (a `.rms` program) to x86-64 assembly in GNU `as` syntax.
///
/// The returned string is a complete translation unit: the compiled program
/// followed by the freestanding runtime. Feed it to `as` and `ld` to obtain an
/// executable. Every error is a human-readable string with a byte offset into
/// `src`.
pub fn compile_to_asm(src: &str, opts: &CompileOptions) -> Result<String, String> {
    let toks = lexer::lex(src)?;
    let mut prog = parser::Parser::new(toks).parse_program()?;
    sema::analyze(&mut prog)?;
    if !opts.no_inline {
        inline::run(&mut prog);
    }

    let mut cg = codegen::Codegen::new();
    if opts.overflow_checks {
        cg = cg.overflow_checks();
    }
    if opts.bounds_checks {
        cg = cg.bounds_checks();
    }
    let user_asm = cg.compile(&prog);

    let mut asm = String::with_capacity(user_asm.len() + runtime::RUNTIME_ASM.len() + 64);
    asm.push_str(&user_asm);
    asm.push_str(runtime::RUNTIME_ASM);
    Ok(asm)
}
