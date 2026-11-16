//! `rmc-mir`: compile a `rustc --emit=mir` file with the rmc backend.
//!
//! Usage: rmcmir <input.mir> -o <output> [--emit-asm]
//!
//! This is the second half of the `rustc front end + rmc back end` pipeline:
//! rustc emits MIR, this driver lowers it to x86-64 and links an ELF
//! executable in-process (no LLVM, no `as`, no `ld`, no libc).

use rmc::mir_codegen::Options;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut emit_asm = false;
    let mut overflow_checks = false;
    let mut no_promote = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-o" => output = Some(PathBuf::from(args.next().expect("-o needs a path"))),
            "--emit-asm" => emit_asm = true,
            "--overflow-checks" => overflow_checks = true,
            "--no-promote" => no_promote = true,
            other => input = Some(PathBuf::from(other)),
        }
    }
    let input = input.expect("usage: rmcmir <input.mir> -o <output>");
    let output = output.unwrap_or_else(|| input.with_extension(""));

    let text = std::fs::read_to_string(&input).expect("cannot read input");
    // `--overflow-checks` still gates the MIR passes below (a hoisted or
    // loop-closed `+` would move a trap); the trap itself is already an
    // `assert` in the input MIR, so the back end needs no switch of its own.
    let opts = Options { promote: !no_promote };
    // A parse failure is the expected outcome for a construct the back end does
    // not support, so it must exit like any other compile error: a clean
    // message and status 1, not a panic and a backtrace note. `tools/fuzz.py`
    // and the differential suite both rely on "unsupported" being a status, not
    // a crash.
    // `RMC_TIME=1` prints parse, each MIR pass, codegen and assembly times.
    let timing = std::env::var_os("RMC_TIME").is_some();
    let phase = |label: &str, t: std::time::Instant| {
        if timing {
            eprintln!("{label:>24} {:8.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    };
    let t = std::time::Instant::now();
    let mut prog = match rmc::mir::parse_with(&text, overflow_checks) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "rmc-mir: error: {e}");
            std::process::exit(1);
        }
    };
    // The same pass list as `rmc --rustc-frontend` and the codegen backend;
    // `RMC_TIME=1` prints per-pass timings.
    phase("parse", t);
    let mo = rmc::MirOptions { overflow_checks, ..rmc::MirOptions::default() };
    rmc::optimize_mir(&mut prog, &mo);
    let t = std::time::Instant::now();
    match rmc::compile_mir_to_asm(&prog, &opts) {
        Ok(asm) => {
            phase("codegen", t);
            let t = std::time::Instant::now();
            if emit_asm {
                print!("{asm}");
                return;
            }
            // An undefined symbol here is a construct the back end lowered to a
            // call it cannot satisfy (a float intrinsic such as `min`, say), so
            // it is an unsupported-input error, not a crash.
            let bytes = match rmc::elf::assemble(&asm) {
                Ok(b) => b,
                Err(e) => {
                    let _ = writeln!(std::io::stderr(), "rmc-mir: error: {e}");
                    std::process::exit(1);
                }
            };
            phase("assemble", t);
            if let Err(e) = std::fs::write(&output, &bytes) {
                let _ = writeln!(std::io::stderr(), "rmc-mir: error: cannot write {}: {e}", output.display());
                std::process::exit(1);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) = std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o755)) {
                    let _ = writeln!(std::io::stderr(), "rmc-mir: error: chmod {}: {e}", output.display());
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "rmc-mir: error: {e}");
            std::process::exit(1);
        }
    }
}
