//! rmc: a small ahead-of-time compiler for a Rust subset.
//!
//! Pipeline: source -> lex -> parse -> semantic analysis -> x86-64 assembly ->
//! ELF64 executable. No LLVM, no external crates, and by default no external
//! assembler or linker: the assembly the code generator emits is assembled and
//! linked in-process. The emitted program links against a freestanding assembly
//! runtime and makes raw Linux syscalls. `--binutils` falls back to GNU `as`
//! and `ld` when the assembly text must be handed to the system tools.

use rmc::{compile_to_asm, CompileOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

struct Options {
    input: PathBuf,
    output: PathBuf,
    emit_asm: bool,
    emit_runtime_asm: bool,
    rustc_frontend: bool,
    keep_mir: bool,
    keep_asm: bool,
    strip: bool,
    min_size: bool,
    binutils: bool,
    overflow_checks: bool,
    bounds_checks: bool,
    no_inline: bool,
    no_licm: bool,
    no_tco: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_asm = false;
    let mut emit_runtime_asm = false;
    let mut rustc_frontend = false;
    let mut keep_mir = false;
    let mut keep_asm = false;
    let mut strip = true;
    let mut min_size = false;
    let mut binutils = false;
    let mut overflow_checks = false;
    let mut bounds_checks = false;
    let mut no_inline = false;
    let mut no_licm = false;
    let mut no_tco = false;

    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => {
                let v = args.next().ok_or("-o requires a path argument")?;
                output = Some(PathBuf::from(v));
            }
            "--emit-asm" => emit_asm = true,
            "--emit-runtime-asm" => emit_runtime_asm = true,
            "--rustc-frontend" => rustc_frontend = true,
            "--keep-mir" => keep_mir = true,
            "--keep-asm" => keep_asm = true,
            "--no-strip" => strip = false,
            "--min-size" => min_size = true,
            "--binutils" => binutils = true,
            "--overflow-checks" => overflow_checks = true,
            "--bounds-checks" => bounds_checks = true,
            "--no-inline" => no_inline = true,
            "--no-licm" => no_licm = true,
            "--no-tco" => no_tco = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                if other.starts_with('-') {
                    return Err(format!("unknown option `{other}`"));
                }
                if input.is_some() {
                    return Err("more than one input file given".into());
                }
                input = Some(PathBuf::from(other));
            }
        }
    }

    // `--emit-runtime-asm` needs no input; every other mode does.
    let input = match input {
        Some(p) => p,
        None if emit_runtime_asm => PathBuf::new(),
        None => return Err("no input file given (see --help)".into()),
    };
    // `--no-strip` asks for a symbol table so `nm`/`gdb` can read the artifact;
    // `--min-size` runs `strip --strip-all`, which removes exactly that table.
    // The two request opposite artifacts, so reject the combination rather than
    // silently letting `--min-size` win.
    if !strip && min_size {
        return Err("--no-strip and --min-size conflict: --min-size strips the symbol table --no-strip keeps".into());
    }
    // `--no-strip` exists so `nm`/`gdb`/`objdump` can read the artifact, which
    // needs a symbol table. The in-process encoder builds none, so the flag
    // selects the binutils path instead of silently producing a symbol-less
    // binary.
    if !strip {
        binutils = true;
    }
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        Path::new(".").join(stem)
    });
    Ok(Options {
        input,
        output,
        emit_asm,
        emit_runtime_asm,
        rustc_frontend,
        keep_mir,
        keep_asm,
        strip,
        min_size,
        binutils,
        overflow_checks,
        bounds_checks,
        no_inline,
        no_licm,
        no_tco,
    })
}

fn print_help() {
    println!(
        "rmc - experimental Rust-subset compiler (no LLVM, no crates)\n\
         \n\
         USAGE:\n    rmc [OPTIONS] <input.rs>    compile with rmc's own front end\n\
         \x20   rmc --rustc-frontend <input.rs>  use rustc's parser and MIR optimiser\n\
         \x20   rmc [OPTIONS] <input.mir>   lower `rustc --emit=mir` text directly\n\
         \n\
         OPTIONS:\n\
         \x20   -o <path>      output binary path\n\
         \x20   --emit-asm     print generated assembly to stdout and stop\n\
         \x20   --emit-runtime-asm  print the freestanding runtime as assembly and stop\n\
         \x20   --rustc-frontend   use rustc's parser and MIR optimiser as the front end\n\
         \x20   --keep-mir     keep the MIR text next to the output (a .rs input)\n\
         \x20   --keep-asm     keep the intermediate .s file next to the output\n\
         \x20   --no-strip     keep the symbol table in the output binary\n\
         \x20   --min-size     with --binutils, also drop the section headers\n\
         \x20   --binutils     use GNU `as` and `ld` instead of linking in-process\n\
         \x20   --overflow-checks  trap on i64 +,-,* overflow (off by default)\n\
         \x20   --bounds-checks    trap on an out-of-range array index (off by default)\n\
         \x20   --no-inline        disable the inlining pass (on by default)\n\
         \x20   --no-licm          disable loop-invariant code motion (on by default)\n\
         \x20   --no-tco           disable self-recursion-to-loop (on by default)\n\
         \x20   -h, --help     show this help\n\
         \n\
         The accepted language is a strict subset of Rust. Entry point is `fn main`."
    );
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rmc: error: {e}");
            std::process::exit(2);
        }
    };

    let t0 = Instant::now();
    if let Err(e) = run(opts) {
        eprintln!("rmc: error: {e}");
        std::process::exit(1);
    }
    // Compile wall time (excludes nothing; used by the benchmark harness).
    if std::env::var_os("RMC_TIMING").is_some() {
        eprintln!("rmc: compile time {:.3} ms", t0.elapsed().as_secs_f64() * 1000.0);
    }
}

/// Drive the rustc front end: run `rustc --emit=mir` and return the MIR text.
/// This is the second pipeline — rustc's parser, type checker and MIR
/// optimiser feed rmc's own x86-64 back end, so the compile is dominated by
/// rmc's in-process assembler rather than by LLVM.
///
/// `-Zno-codegen` is what makes this cheap: it tells rustc to stop after the
/// MIR optimisations and skip the monomorphisation and codegen preparation
/// that `--emit=mir` throws away anyway. rmc *is* the code generator here, so
/// none of that work is needed, and the MIR it prints is byte-for-byte
/// identical. The flag is nightly-only, so `RUSTC_BOOTSTRAP=1` is set on the
/// child (never on rmc itself) and the run falls back to plain rustc if the
/// flag is rejected, which keeps a stock stable toolchain working. `-O` stays:
/// dropping it triples the run time of the emitted code (`fib` 5.4 ms ->
/// 20.4 ms), because the MIR-level optimisations it runs are the ones rmc's
/// own passes build on.
fn front_end_mir(input: &Path) -> Result<String, String> {
    match run_rustc_front_end(input, true) {
        Ok(mir) => Ok(mir),
        Err(_) => run_rustc_front_end(input, false),
    }
}

/// One rustc front-end invocation. `no_codegen` selects `-Zno-codegen` (with
/// `RUSTC_BOOTSTRAP=1`); the fallback call passes `false`.
fn run_rustc_front_end(input: &Path, no_codegen: bool) -> Result<String, String> {
    let mut cmd = Command::new("rustc");
    cmd.arg("--emit=mir")
        .arg("-o")
        .arg("-")
        .arg("-O")
        .arg("-C")
        .arg("panic=abort")
        .arg("--crate-type=bin")
        .arg("--edition")
        .arg("2021");
    if no_codegen {
        cmd.arg("-Zno-codegen").env("RUSTC_BOOTSTRAP", "1");
    }
    let out = cmd
        .arg(input)
        .output()
        .map_err(|e| format!("failed to run `rustc`: {e}"))?;
    if !out.status.success() {
        // The first (flagged) attempt stays quiet: if it failed because the
        // toolchain rejected `-Zno-codegen`, the fallback prints the real
        // diagnostic; if it failed on the source, the fallback prints it once.
        if !no_codegen {
            let _ = std::io::stderr().write_all(&out.stderr);
        }
        return Err("rustc front end failed".into());
    }
    let mir = out.stdout;
    if mir.is_empty() {
        return Err("rustc emitted no MIR".into());
    }
    Ok(String::from_utf8_lossy(&mir).into_owned())
}

/// Compile a `.rs` input through the rustc-MIR front end and the rmc back end.
fn run_mir_pipeline(opts: &Options) -> Result<(), String> {
    let mir = front_end_mir(&opts.input)?;
    if opts.keep_mir {
        let mir_path = opts.output.with_extension("mir");
        std::fs::write(&mir_path, &mir)
            .map_err(|e| format!("cannot write {}: {e}", mir_path.display()))?;
    }
    run_mir_text(opts, &mir)
}

/// Lower `rustc --emit=mir` text with the rmc back end. Shared by the `.rs`
/// front end and the `.mir` input path, so both go through the same passes.
fn run_mir_text(opts: &Options, mir: &str) -> Result<(), String> {
    let mut prog = rmc::mir::parse(mir)?;
    if !opts.no_tco {
        rmc::mir::tco(&mut prog, opts.overflow_checks);
    }
    if !opts.no_inline {
        rmc::mir::inline_calls(&mut prog);
    }
    rmc::mir::coalesce_call_results(&mut prog);
    rmc::mir::merge_blocks(&mut prog);
    prog.copy_propagate();
    rmc::mir::drop_redundant_asserts(&mut prog);
    if !opts.no_licm {
        rmc::mir::licm(&mut prog);
    }
    rmc::mir::fold_accumulators(&mut prog);
    rmc::mir::close_counted_loops(&mut prog, opts.overflow_checks);
    rmc::mir::jump_thread(&mut prog);
    rmc::mir::rotate_loops(&mut prog);
    rmc::mir::layout_blocks(&mut prog);
    let mir_opts = rmc::mir_codegen::Options {
        overflow_checks: opts.overflow_checks,
        promote: true,
    };
    let asm = rmc::compile_mir_to_asm(&prog, &mir_opts)?;
    emit_binary(opts, &asm)
}

/// Assemble and link `asm` into `opts.output`, honouring `--emit-asm`,
/// `--binutils`, `--no-strip`, `--min-size` and `--keep-asm`. Shared by the
/// `.rms`/AST path and the MIR path, so every input kind accepts the same
/// output options instead of the MIR path silently ignoring them.
fn emit_binary(opts: &Options, asm: &str) -> Result<(), String> {
    if opts.emit_asm {
        print!("{asm}");
        return Ok(());
    }
    if !opts.binutils {
        // Assemble and link in-process. This is the default: it removes the two
        // binutils processes that dominate compile time and produces a smaller
        // executable (no symbol or section-header tables to begin with).
        let bytes = rmc::elf::assemble(asm)?;
        std::fs::write(&opts.output, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", opts.output.display()))?;
        if opts.keep_asm {
            std::fs::write(opts.output.with_extension("s"), asm)
                .map_err(|e| format!("cannot write assembly: {e}"))?;
        }
        // `ld` marks its output executable; the in-process path must too.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&opts.output, mode)
                .map_err(|e| format!("cannot chmod {}: {e}", opts.output.display()))?;
        }
        return Ok(());
    }

    let asm_path = opts.output.with_extension("s");
    std::fs::write(&asm_path, asm).map_err(|e| format!("cannot write {}: {e}", asm_path.display()))?;

    let obj_path = opts.output.with_extension("o");
    assemble(&asm_path, &obj_path)?;
    link(&obj_path, &opts.output, opts.strip)?;
    if opts.min_size {
        strip_section_headers(&opts.output)?;
    }

    if !opts.keep_asm {
        let _ = std::fs::remove_file(&asm_path);
    }
    let _ = std::fs::remove_file(&obj_path);
    Ok(())
}

fn run(opts: Options) -> Result<(), String> {
    // Emit the freestanding runtime alone. The differential test suite uses
    // this to link a rustc reference against the same runtime rmc uses, so the
    // two binaries differ only in the code generator.
    if opts.emit_runtime_asm {
        print!("{}", rmc::runtime::support_asm());
        return Ok(());
    }
    // `--rustc-frontend` routes a `.rs` input through rustc's parser and MIR
    // optimiser instead of rmc's own front end. Without it a `.rs` file is
    // rmc's own source language, compiled end to end (the default pipeline), so
    // the flag is what distinguishes the two meanings of a `.rs` input.
    if opts.rustc_frontend && opts.input.extension().and_then(|e| e.to_str()) == Some("rs") {
        return run_mir_pipeline(&opts);
    }
    // A `.mir` input is already MIR text (as `rustc --emit=mir` prints it), so
    // it skips the front end and enters the back end directly. This is the
    // entry point the MIR fuzzer mutates into.
    if opts.input.extension().and_then(|e| e.to_str()) == Some("mir") {
        let mir = std::fs::read_to_string(&opts.input)
            .map_err(|e| format!("cannot read {}: {e}", opts.input.display()))?;
        return run_mir_text(&opts, &mir);
    }
    let src = std::fs::read_to_string(&opts.input)
        .map_err(|e| format!("cannot read {}: {e}", opts.input.display()))?;

    let asm = compile_to_asm(
        &src,
        &CompileOptions {
            overflow_checks: opts.overflow_checks,
            bounds_checks: opts.bounds_checks,
            no_inline: opts.no_inline,
            no_licm: opts.no_licm,
            no_tco: opts.no_tco,
        },
    )?;
    emit_binary(&opts, &asm)
}

fn assemble(asm: &Path, obj: &Path) -> Result<(), String> {
    let out = Command::new("as")
        .arg("--64")
        .arg("-o")
        .arg(obj)
        .arg(asm)
        .output()
        .map_err(|e| format!("failed to run `as`: {e}"))?;
    if !out.status.success() {
        let _ = std::io::stderr().write_all(&out.stderr);
        return Err("assembler failed".into());
    }
    Ok(())
}

fn link(obj: &Path, exe: &Path, strip: bool) -> Result<(), String> {
    // The assembly carries a `.note.GNU-stack` section, so the stack is marked
    // non-executable without an explicit `-z noexecstack`.
    //
    // `-z noseparate-code` packs the read-only and executable sections into a
    // single read+execute LOAD segment and keeps the writable data in its own
    // read+write one. That is both smaller than `-n` (which packs *everything*
    // into one read+write+execute segment, an avoidable W^X hazard) and the
    // smallest layout `ld` offers here: 3304 bytes vs 7344 for the hello case.
    let mut cmd = Command::new("ld");
    cmd.arg("-o")
        .arg(exe)
        .arg("--build-id=none")
        .arg("-z")
        .arg("noseparate-code");
    // Dropping the symbol and section-name tables is worth about a third of
    // the artifact at this size (1376 -> 888 bytes on `hello`) and changes
    // nothing the loader reads. `--no-strip` keeps them for `nm`/`gdb`.
    if strip {
        cmd.arg("--strip-all");
    }
    let out = cmd
        .arg(obj)
        .output()
        .map_err(|e| format!("failed to run `ld`: {e}"))?;
    if !out.status.success() {
        let _ = std::io::stderr().write_all(&out.stderr);
        return Err("linker failed".into());
    }
    Ok(())
}

/// `--min-size`: run `strip --strip-section-headers` on the linked binary.
///
/// The section-header table is not read by the loader, so dropping it is safe
/// and saves another 40% of the file (888 -> 536 bytes on `hello`). It needs a
/// second binutils process (~2.3 ms), which is a third of rmc's own compile
/// time, so it is opt-in rather than the default. `strip` ships with `as`/`ld`,
/// so it adds no new dependency. The binary still runs and `readelf -lW` still
/// works, but `nm`/`objdump -d` need the section table, so this is for a
/// deployment artifact, not a debugging one.
fn strip_section_headers(exe: &Path) -> Result<(), String> {
    let out = Command::new("strip")
        .arg("--strip-all")
        .arg("--strip-section-headers")
        .arg(exe)
        .output()
        .map_err(|e| format!("failed to run `strip`: {e}"))?;
    if !out.status.success() {
        let _ = std::io::stderr().write_all(&out.stderr);
        return Err("strip failed".into());
    }
    Ok(())
}
