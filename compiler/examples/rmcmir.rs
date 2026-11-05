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
    let opts = Options { overflow_checks, promote: !no_promote, align_loops: true };
    let mut prog = rmc::mir::parse(&text).expect("mir parse");
    rmc::mir::inline_calls(&mut prog);
    rmc::mir::merge_blocks(&mut prog);
    prog.copy_propagate();
    rmc::mir::licm(&mut prog);
    rmc::mir::jump_thread(&mut prog);
    match rmc::compile_mir_to_asm(&prog, &opts) {
        Ok(asm) => {
            if emit_asm {
                print!("{asm}");
                return;
            }
            let bytes = rmc::elf::assemble(&asm).expect("assemble");
            std::fs::write(&output, &bytes).expect("write output");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o755))
                    .expect("chmod");
            }
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "rmc-mir: error: {e}");
            std::process::exit(1);
        }
    }
}
