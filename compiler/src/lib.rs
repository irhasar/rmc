//! rmc: an ahead-of-time compiler for a small, statically-typed subset of Rust.
//!
//! The pipeline is lex -> parse -> semantic analysis -> inline -> self-recursion
//! to loop -> loop-invariant code motion -> x86-64 assembly. This library exposes it as a single call,
//! [`compile_to_asm`], so rmc can be embedded in another program instead of only
//! run as a command-line tool. The `rmc` binary in `src/main.rs` drives this
//! library and then invokes `as` and `ld`; those two are the only external
//! dependencies, and neither is needed if you only want the assembly text.
//!
//! ```
//! let asm = rmc::compile_to_asm("fn main() -> i64 { return 0; }", &Default::default())
//!     .expect("compiles");
//! assert!(asm.contains("main:"));
//! ```

pub mod ast;
pub mod codegen;
pub mod consts;
pub mod dce;
pub mod elf;
pub mod inline;
pub mod licm;
pub mod lexer;
pub mod mir;
pub mod mir_codegen;
pub mod parser;
pub mod runtime;
pub mod sema;
pub mod tco;

/// Options that change what the compiler emits.
#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    /// Insert a trap after every `+`, `-` and `*` so that signed overflow exits
    /// with status 101 instead of wrapping. Off by default because it costs a
    /// branch per arithmetic operation.
    pub overflow_checks: bool,
    /// Disable the bounds check before every runtime index. Checks are **on**
    /// by default: an out-of-range `a[i]` / `v[i]` traps with exit status 101
    /// and `rmc: index out of bounds`, matching rustc, instead of reading or
    /// writing memory the program does not own. This flag is the explicit
    /// unsafe opt-out; it exists for the rare program that has already proven
    /// its indices in range and wants the compare and branch gone.
    pub no_bounds_checks: bool,
    /// Skip the inlining pass. Inlining is on by default: it is what makes a
    /// call to a small accessor as cheap as the arithmetic it wraps.
    pub no_inline: bool,
    /// Skip loop-invariant code motion. It is on by default and hoists
    /// invariant arithmetic out of a loop body.
    pub no_licm: bool,
    /// Skip the self-recursion-to-loop pass. It is on by default and halves the
    /// call tree of a `f(x) = f(g(x)) op f(h(x))` function such as `fib`.
    pub no_tco: bool,
}

/// Compile `src` (a `.rs` program) to x86-64 assembly in GNU `as` syntax.
///
/// The returned string is a complete translation unit: the compiled program
/// followed by the freestanding runtime. Feed it to `as` and `ld` to obtain an
/// executable. Every error is a human-readable string with a byte offset into
/// `src`.
pub fn compile_to_asm(src: &str, opts: &CompileOptions) -> Result<String, String> {
    let toks = lexer::lex(src)?;
    let mut prog = parser::Parser::new(toks)
        .bounds_checks(!opts.no_bounds_checks)
        .parse_program()?;
    sema::analyze(&mut prog)?;
    if !opts.no_inline {
        inline::run(&mut prog);
    }
    if !opts.no_tco {
        tco::run(&mut prog, opts.overflow_checks);
    }
    if !opts.no_licm {
        licm::run(&mut prog, opts.overflow_checks);
    }
    dce::run(&mut prog);

    let mut cg = codegen::Codegen::new();
    if opts.overflow_checks {
        cg = cg.overflow_checks();
    }
    if !opts.no_bounds_checks {
        cg = cg.bounds_checks();
    }
    let user_asm = cg.compile(&prog);

    let runtime = runtime::for_program(&user_asm);
    let mut asm = String::with_capacity(user_asm.len() + runtime.len() + 64);
    asm.push_str(&user_asm);
    asm.push_str(&runtime);
    Ok(asm)
}

/// Compile a parsed MIR program to x86-64 assembly (program plus runtime).
///
/// The MIR path shares the freestanding runtime and the in-process assembler
/// with [`compile_to_asm`]; only the front end (rustc's MIR) and the lowering
/// (`mir_codegen`) differ.
pub fn compile_mir_to_asm(
    prog: &mir::Program,
    opts: &mir_codegen::Options,
) -> Result<String, String> {
    let user_asm = mir_codegen::compile(prog, opts)?;
    let runtime = runtime::for_program(&user_asm);
    let mut asm = String::with_capacity(user_asm.len() + runtime.len() + 64);
    asm.push_str(&user_asm);
    asm.push_str(&runtime);
    Ok(asm)
}
