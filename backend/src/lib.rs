//! rmc as a rustc codegen backend (`-Zcodegen-backend`).
//!
//! This is the architecturally correct interface the printed-MIR prototype
//! (`rmc --rustc-frontend`) stands in for: rustc links this dylib and hands it
//! the *same* MIR data structures the LLVM backend would see, with no
//! `--emit=mir` text round-trip and no second `rustc` process.
//!
//! The lowering is rmc's existing one. `codegen_crate` renders the optimised
//! MIR with rustc's own pretty printer (the identical function behind
//! `--emit=mir`, so the text is byte-for-byte what the prototype parsed),
//! feeds it through `rmc::mir`, and writes the finished ELF directly. Linking
//! is rmc's in-process encoder, so `join_codegen` and `link` are no-ops.
//!
//! Build: `backend/build.sh` (nightly + `rustc-dev`). Use:
//!   rustc -Zcodegen-backend=<...>/libr..._codegen_backend.so foo.rs

#![feature(rustc_private)]

extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_errors;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_structures;
extern crate rustc_metadata;
extern crate rustc_span;
use rustc_span::Symbol;
extern crate rustc_target;
use rustc_target::spec::Arch;

use rustc_codegen_ssa::traits::CodegenBackend;
use rustc_codegen_ssa::TargetConfig;
use rustc_codegen_ssa::{CompiledModules, CrateInfo};
use rustc_data_structures::unord::{UnordMap, UnordSet};
use rustc_middle::dep_graph::{WorkProduct, WorkProductId};
use rustc_middle::mir::write_mir_pretty;
use rustc_middle::ty::TyCtxt;
use rustc_session::config::{OutputFilenames, OutputType, OutFileName};
use rustc_structures::CrateType;
use rustc_session::{EarlySession, Session, IncrCompSession};
use std::any::Any;
use std::io::Write;
use std::path::Path;

#[derive(Clone)]
struct RmcBackend;

/// The lowering, factored out so it is callable (and testable) without a
/// `TyCtxt`. `mir_text` is exactly what `--emit=mir` would print.
///
/// `overflow_checks` must be the session's setting: the MIR dump contains the
/// checked-arithmetic shapes only under `-C overflow-checks=on`, and both the
/// parser's `switchInt(OverflowChecks)` fold and the loop rewrites below change
/// behavior with it.
fn compile_mir_text(mir_text: &str, overflow_checks: bool) -> Result<Vec<u8>, String> {
    let mut prog = rmc::mir::parse_with(mir_text, overflow_checks)
        .map_err(|e| format!("rmc-mir: error: {e}"))?;
    let opts = rmc::mir_codegen::Options { promote: true };
    rmc::mir::tco(&mut prog, overflow_checks);
    rmc::mir::inline_calls(&mut prog);
    rmc::mir::coalesce_call_results(&mut prog);
    rmc::mir::merge_blocks(&mut prog);
    prog.copy_propagate();
    rmc::mir::drop_redundant_asserts(&mut prog);
    rmc::mir::licm(&mut prog);
    rmc::mir::fold_accumulators(&mut prog);
    rmc::mir::strength_reduce(&mut prog);
    rmc::mir::close_counted_loops(&mut prog, overflow_checks);
    rmc::mir::jump_thread(&mut prog);
    rmc::mir::rotate_loops(&mut prog);
    rmc::mir::layout_blocks(&mut prog);
    let asm = rmc::compile_mir_to_asm(&prog, &opts)?;
    rmc::elf::assemble(&asm)
}

impl CodegenBackend for RmcBackend {
    fn name(&self) -> &'static str {
        "rmc"
    }

    /// Report the target features the backend implements.
    ///
    /// rustc asks the *backend* which target features are available, because
    /// only the backend knows. With no answer, `cfg(target_feature)` is empty
    /// and the ABI check in `check_abi_required_features` cannot see `x87` or
    /// `sse2`, so it warns (and, on x86, will soon error) that the ABI cannot
    /// be implemented. rmc emits plain x86-64 for the default hard-float ABI,
    /// so it declares the x86-64 baseline feature set that `rustc --print cfg`
    /// reports for this target.
    fn target_config(&self, sess: &EarlySession) -> TargetConfig {
        let mut internal_target_features = UnordSet::default();
        // `sess.target.features` is empty for x86_64-linux-gnu (LLVM infers the
        // baseline from the CPU), so the baseline is spelled out here.
        if sess.target.arch == Arch::X86_64 {
            for f in ["x87", "sse", "sse2", "fxsr"] {
                internal_target_features.insert(Symbol::intern(f));
            }
        }
        TargetConfig {
            internal_target_features,
            // rmc has no `f16`/`f128` arithmetic, so it must not advertise the
            // corresponding `cfg(target_has_reliable_*)` guarantees.
            has_reliable_f16: false,
            has_reliable_f16_math: false,
            has_reliable_f16b: false,
            has_reliable_f128: false,
            has_reliable_f128_math: false,
        }
    }

    fn target_cpu(&self, _sess: &Session) -> String {
        "generic".to_string()
    }

    fn codegen_crate<'tcx>(&self, tcx: TyCtxt<'tcx>) -> Box<dyn Any> {
        // `assemble` only ever writes x86-64 ELF, so a cross-target invocation
        // would silently produce an executable for the wrong machine.
        if tcx.sess.target.arch != Arch::X86_64 {
            tcx.dcx().fatal(format!(
                "rmc codegen backend only supports the x86-64 target, not `{}`",
                tcx.sess.target.llvm_target
            ));
        }

        // Only an executable is in scope: rmc emits a complete freestanding
        // ELF (its own runtime and `_start`), so there is nothing to link
        // afterwards and no object file to hand back. Reject `--emit=obj`,
        // `--emit=asm` and the other codegen modes explicitly instead of
        // writing an executable to the path they asked for.
        let crate_types = tcx.crate_types();
        if !crate_types.contains(&CrateType::Executable) {
            tcx.dcx().fatal("rmc codegen backend only supports `--crate-type=bin`");
        }
        for ty in tcx.sess.opts.output_types.keys() {
            if *ty != OutputType::Exe && *ty != OutputType::Metadata && *ty != OutputType::DepInfo {
                tcx.dcx().fatal(format!(
                    "rmc codegen backend cannot emit `{}`; only an executable is supported",
                    ty.shorthand()
                ));
            }
        }

        let mut mir_bytes: Vec<u8> = Vec::new();
        if let Err(e) = write_mir_pretty(tcx, &mut mir_bytes) {
            tcx.dcx().fatal(format!("rmc: could not render MIR: {e}"));
        }
        // Rendering the MIR walks `trimmed_def_paths`, which arms a
        // "must produce a diagnostic" guard: rustc assumes that any pass
        // needing trimmed paths is printing something the user will read
        // (`--emit=mir`, `-Zunpretty`, `-Zdump-mir`, ...). A codegen backend
        // renders MIR as an implementation detail, so the guard is cleared
        // here — otherwise rustc's `DiagCtxt` teardown panics.
        tcx.dcx().reset_err_count();
        if let Some(path) = std::env::var_os("RMC_DUMP_MIR") {
            let _ = std::fs::write(&path, &mir_bytes);
        }
        let mir_text = String::from_utf8_lossy(&mir_bytes);

        let bytes = match compile_mir_text(&mir_text, tcx.sess.overflow_checks()) {
            Ok(b) => b,
            Err(e) => tcx.dcx().fatal(e),
        };

        let out = match tcx.output_filenames(()).path(OutputType::Exe) {
            OutFileName::Real(p) => p,
            OutFileName::Stdout => {
                tcx.dcx().fatal("rmc codegen backend cannot write an executable to stdout");
            }
        };
        if let Err(e) = write_executable(&out, &bytes) {
            tcx.dcx().fatal(format!("rmc: cannot write {}: {e}", out.display()));
        }

        Box::new(())
    }

    fn join_codegen(
        &self,
        _ongoing: Box<dyn Any>,
        _sess: &Session,
        _incr: Option<&IncrCompSession>,
        _outputs: &OutputFilenames,
        _crate_info: &CrateInfo,
    ) -> (CompiledModules, UnordMap<WorkProductId, WorkProduct>) {
        // Everything is finished in `codegen_crate`; no object files, no
        // allocator module, nothing for rustc to link.
        (
            CompiledModules { modules: Vec::new(), allocator_module: None },
            UnordMap::default(),
        )
    }

    fn link(
        &self,
        _sess: &Session,
        _compiled_modules: CompiledModules,
        _crate_info: CrateInfo,
        _metadata: rustc_metadata::EncodedMetadata,
        _outputs: &OutputFilenames,
    ) {
    }
}

fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    drop(f);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[no_mangle]
pub extern "Rust" fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(RmcBackend)
}
