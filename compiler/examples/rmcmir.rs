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
    let timing = std::env::var_os("RMC_TIME").is_some();
    let mut t0 = std::time::Instant::now();
    let mark = |label: &str, t0: &mut std::time::Instant| {
        if timing {
            eprintln!("{:>22} {:8.1} ms", label, t0.elapsed().as_secs_f64() * 1000.0);
            *t0 = std::time::Instant::now();
        }
    };
    // A parse failure is the expected outcome for a construct the back end does
    // not support, so it must exit like any other compile error: a clean
    // message and status 1, not a panic and a backtrace note. `tools/fuzz.py`
    // and the differential suite both rely on "unsupported" being a status, not
    // a crash.
    let mut prog = match rmc::mir::parse_with(&text, overflow_checks) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "rmc-mir: error: {e}");
            std::process::exit(1);
        }
    };
    mark("parse", &mut t0);
    rmc::mir::tco(&mut prog, overflow_checks);
    mark("tco", &mut t0);
    rmc::mir::inline_calls(&mut prog);
    mark("inline", &mut t0);
    rmc::mir::coalesce_call_results(&mut prog);
    mark("coalesce", &mut t0);
    rmc::mir::merge_blocks(&mut prog);
    mark("merge_blocks", &mut t0);
    prog.copy_propagate();
    mark("copy_propagate", &mut t0);
    rmc::mir::drop_redundant_asserts(&mut prog);
    mark("drop_asserts", &mut t0);
    rmc::mir::licm(&mut prog);
    mark("licm", &mut t0);
    rmc::mir::fold_accumulators(&mut prog);
    rmc::mir::strength_reduce(&mut prog);
    rmc::mir::close_counted_loops(&mut prog, overflow_checks);
    mark("fold_accumulators", &mut t0);
    rmc::mir::jump_thread(&mut prog);
    mark("jump_thread", &mut t0);
    rmc::mir::rotate_loops(&mut prog);
    mark("rotate_loops", &mut t0);
    rmc::mir::layout_blocks(&mut prog);
    mark("layout_blocks", &mut t0);
    match rmc::compile_mir_to_asm(&prog, &opts) {
        Ok(asm) => {
            mark("codegen", &mut t0);
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
            mark("assemble", &mut t0);
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
