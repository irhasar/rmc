//! rmc: an ahead-of-time compiler for a statically-typed subset of Rust.
//!
//! Two routes share one x86-64 back end:
//!
//! * rmc's own front end ([`compile_to_asm`]): lex -> parse (with desugaring)
//!   -> semantic analysis -> AST passes (inline, self-recursion to loop,
//!   loop-invariant code motion, dead-function elimination) -> assembly;
//! * rustc's front end ([`mir_text_to_asm`], used by `rmc --rustc-frontend`,
//!   the `rmcmir` example and the `-Zcodegen-backend` dylib): rustc's MIR ->
//!   [`optimize_mir`] -> assembly.
//!
//! Either way the result is a complete translation unit (program plus the
//! freestanding runtime) that [`elf::assemble`] turns into a static ELF64
//! executable in-process; GNU `as` and `ld` are needed only for `--binutils`.
//!
//! ```
//! let asm = rmc::compile_to_asm("fn main() -> i64 { return 0; }", &Default::default())
//!     .expect("compiles");
//! assert!(asm.contains("main:"));
//! ```

// Source layout: `front` (rmc's own front end), `opt` (AST passes), `mir`
// (the rustc-MIR route: parser, passes and lowering) and `x86` (the code
// generator, runtime and in-process assembler/linker both routes share).
pub mod front;
pub mod mir;
pub mod opt;
pub mod x86;

// The flat names every module (and the backend crate, the examples and the
// tests) used before the split, kept so a path like `crate::ast::Expr` or
// `rmc::mir_codegen::Options` still resolves.
pub use front::{ast, consts, lexer, parser, sema};
pub use mir::codegen as mir_codegen;
pub use opt::{dce, inline, licm, tco};
pub use x86::{codegen, elf, runtime};

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

/// The largest stack frame a function may ask for, in bytes. Linux gives the
/// main thread an 8 MiB stack by default; a frame near that size faults on its
/// first touch. rmc stores every array element in its own 8-byte slot, so
/// `[u8; 1 << 20]` (1 MiB in rustc) needs an 8 MiB frame here.
const MAX_FRAME_BYTES: i64 = 6 << 20;

/// Reject a function whose frame cannot fit the default stack, with a message
/// that says why, instead of emitting a binary that segfaults on entry.
fn check_frame_sizes(prog: &ast::Program) -> Result<(), String> {
    for f in &prog.fns {
        let bytes = f.nslots as i64 * 8;
        if bytes > MAX_FRAME_BYTES {
            return Err(format!(
                "function `{}` needs a {:.1} MiB stack frame, more than the {} MiB rmc allows: \
                 rmc stores every array element in an 8-byte slot (a `[u8; N]` takes 8*N bytes), \
                 so a large local array does not fit the default 8 MiB stack; \
                 allocate it with `vec![..]` instead",
                f.name.split('$').last().unwrap_or(&f.name),
                bytes as f64 / (1 << 20) as f64,
                MAX_FRAME_BYTES >> 20
            ));
        }
    }
    Ok(())
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
    check_frame_sizes(&prog)?;

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

/// Settings for the MIR pipeline. The `rmc` CLI (`--rustc-frontend`, `.mir`
/// input) and the `-Zcodegen-backend` dylib both build one of these and call
/// [`optimize_mir`], so the two routes run exactly the same passes. They used
/// to carry two hand-copied pass lists that drifted apart (the dylib skipped
/// `prune_unreachable`, and the CLI parsed with overflow checks always off).
#[derive(Clone, Debug)]
pub struct MirOptions {
    /// The crate was built with `-C overflow-checks=on`: selects the checked
    /// arm of `switchInt(OverflowChecks)` and disables reassociating passes.
    pub overflow_checks: bool,
    pub inline: bool,
    pub licm: bool,
    pub tco: bool,
}

impl Default for MirOptions {
    fn default() -> Self {
        MirOptions { overflow_checks: false, inline: true, licm: true, tco: true }
    }
}

/// Run the MIR optimisation pipeline over `prog`.
///
/// Debugging switches, read from the environment so a miscompile can be
/// bisected without rebuilding:
///
/// * `RMC_SKIP_MIR_PASSES=a,b` skips the named passes (the names below);
/// * `RMC_TIME=1` prints each pass's wall time to stderr.
pub fn optimize_mir(prog: &mut mir::Program, o: &MirOptions) {
    let skip = std::env::var("RMC_SKIP_MIR_PASSES").unwrap_or_default();
    let timing = std::env::var_os("RMC_TIME").is_some();
    let ovf = o.overflow_checks;
    type Pass<'a> = (&'static str, bool, &'a dyn Fn(&mut mir::Program));
    let passes: [Pass; 19] = [
        ("tco", o.tco, &|p| mir::tco(p, ovf)),
        ("inline", o.inline, &mir::inline_calls),
        ("coalesce_call_results", true, &mir::coalesce_call_results),
        ("merge_blocks", true, &mir::merge_blocks),
        ("scalarize_aggregates", true, &mir::scalarize_aggregates),
        ("select_diamonds", true, &mir::select_diamonds),
        ("copy_propagate", true, &|p| p.copy_propagate()),
        ("drop_redundant_asserts", true, &mir::drop_redundant_asserts),
        ("licm", o.licm, &mir::licm),
        ("fold_accumulators", true, &mir::fold_accumulators),
        ("strength_reduce", true, &mir::strength_reduce),
        ("close_counted_loops", true, &|p| mir::close_counted_loops(p, ovf)),
        ("jump_thread", true, &mir::jump_thread),
        ("prune_unreachable", true, &mir::prune_unreachable),
        // After `drop_redundant_asserts`, which reads bounds off the tests.
        ("fold_range_checks", true, &mir::fold_range_checks),
        ("rotate_loops", true, &mir::rotate_loops),
        // Rotation leaves a loop's guard test in a block of its own behind the
        // block that initialises the loop's variables; merged, the next pass
        // can order the saves and the initialisers within one block.
        ("merge_blocks_late", true, &mir::merge_blocks),
        ("split_loop_ranges", true, &mir::split_loop_ranges),
        ("layout_blocks", true, &mir::layout_blocks),
    ];
    for (name, enabled, run) in passes {
        if !enabled || skip.split(',').any(|s| s.trim() == name) {
            continue;
        }
        let t = std::time::Instant::now();
        run(prog);
        if timing {
            eprintln!("{name:>24} {:8.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

/// Parse `rustc --emit=mir` text, optimise it, and lower it to assembly
/// (program plus runtime). The single entry point both MIR routes use.
pub fn mir_text_to_asm(text: &str, o: &MirOptions) -> Result<String, String> {
    let mut prog = mir::parse_with(text, o.overflow_checks)?;
    optimize_mir(&mut prog, o);
    compile_mir_to_asm(&prog, &mir_codegen::Options { promote: true })
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
