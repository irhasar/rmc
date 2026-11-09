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

    let input = input.ok_or("no input file given (see --help)")?;
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
         USAGE:\n    rmc [OPTIONS] <input.rms>\n\
         \n\
         OPTIONS:\n\
         \x20   -o <path>      output binary path\n\
         \x20   --emit-asm     print generated assembly to stdout and stop\n\
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

fn run(opts: Options) -> Result<(), String> {
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

    if opts.emit_asm {
        print!("{asm}");
        return Ok(());
    }

    if !opts.binutils {
        // Assemble and link in-process. This is the default: it removes the two
        // binutils processes that dominate compile time and produces a smaller
        // executable (no symbol or section-header tables to begin with).
        let bytes = rmc::elf::assemble(&asm)?;
        std::fs::write(&opts.output, &bytes)
            .map_err(|e| format!("cannot write {}: {e}", opts.output.display()))?;
        if opts.keep_asm {
            let asm_path = opts.output.with_extension("s");
            std::fs::write(&asm_path, &asm)
                .map_err(|e| format!("cannot write {}: {e}", asm_path.display()))?;
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
    std::fs::write(&asm_path, &asm).map_err(|e| format!("cannot write {}: {e}", asm_path.display()))?;

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
