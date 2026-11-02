//! nrc: a small ahead-of-time compiler for a Rust subset.
//!
//! Pipeline: source -> lex -> parse -> semantic analysis -> x86-64 assembly ->
//! GNU as -> GNU ld. No LLVM, no external crates. The emitted program links
//! against a freestanding assembly runtime and makes raw Linux syscalls.

use nrc::{compile_to_asm, CompileOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

struct Options {
    input: PathBuf,
    output: PathBuf,
    emit_asm: bool,
    keep_asm: bool,
    overflow_checks: bool,
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_asm = false;
    let mut keep_asm = false;
    let mut overflow_checks = false;

    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => {
                let v = args.next().ok_or("-o requires a path argument")?;
                output = Some(PathBuf::from(v));
            }
            "--emit-asm" => emit_asm = true,
            "--keep-asm" => keep_asm = true,
            "--overflow-checks" => overflow_checks = true,
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
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().unwrap_or_default();
        Path::new(".").join(stem)
    });
    Ok(Options {
        input,
        output,
        emit_asm,
        keep_asm,
        overflow_checks,
    })
}

fn print_help() {
    println!(
        "nrc - experimental Rust-subset compiler (no LLVM, no crates)\n\
         \n\
         USAGE:\n    nrc [OPTIONS] <input.nrs>\n\
         \n\
         OPTIONS:\n\
         \x20   -o <path>      output binary path\n\
         \x20   --emit-asm     print generated assembly to stdout and stop\n\
         \x20   --keep-asm     keep the intermediate .s file next to the output\n\
         \x20   --overflow-checks  trap on i64 +,-,* overflow (off by default)\n\
         \x20   -h, --help     show this help\n\
         \n\
         The accepted language is a strict subset of Rust. Entry point is `fn main`."
    );
}

fn main() {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("nrc: error: {e}");
            std::process::exit(2);
        }
    };

    let t0 = Instant::now();
    if let Err(e) = run(opts) {
        eprintln!("nrc: error: {e}");
        std::process::exit(1);
    }
    // Compile wall time (excludes nothing; used by the benchmark harness).
    if std::env::var_os("NRC_TIMING").is_some() {
        eprintln!("nrc: compile time {:.3} ms", t0.elapsed().as_secs_f64() * 1000.0);
    }
}

fn run(opts: Options) -> Result<(), String> {
    let src = std::fs::read_to_string(&opts.input)
        .map_err(|e| format!("cannot read {}: {e}", opts.input.display()))?;

    let asm = compile_to_asm(
        &src,
        &CompileOptions {
            overflow_checks: opts.overflow_checks,
        },
    )?;

    if opts.emit_asm {
        print!("{asm}");
        return Ok(());
    }

    let asm_path = opts.output.with_extension("s");
    std::fs::write(&asm_path, &asm).map_err(|e| format!("cannot write {}: {e}", asm_path.display()))?;

    let obj_path = opts.output.with_extension("o");
    assemble(&asm_path, &obj_path)?;
    link(&obj_path, &opts.output)?;

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

fn link(obj: &Path, exe: &Path) -> Result<(), String> {
    let out = Command::new("ld")
        .arg("-o")
        .arg(exe)
        .arg("-z")
        .arg("noexecstack")
        .arg("--build-id=none")
        .arg("-n")
        .arg(obj)
        .output()
        .map_err(|e| format!("failed to run `ld`: {e}"))?;
    if !out.status.success() {
        let _ = std::io::stderr().write_all(&out.stderr);
        return Err("linker failed".into());
    }
    Ok(())
}
