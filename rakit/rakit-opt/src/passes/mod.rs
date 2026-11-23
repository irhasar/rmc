//! First-party optimization passes and their `Pass` adapters.

pub mod div_test;
pub mod if_convert;
pub mod inliner;
pub mod rec_loop;
pub mod sincos;
pub mod store_forward;
pub mod unroll;

use rakit_ir::backend::{InstructionSelector, RegisterAllocator};

use crate::pass::{Pass, PassCtx};
use crate::passtime;

fn verified(rir: &rakit_ir::Function, label: &str, debug: bool) -> bool {
    if !debug && std::env::var_os("RAKIT_RIR_VERIFY").is_none() {
        return true;
    }
    match rakit_ir::verify(rir) {
        Ok(()) => true,
        Err(errors) => {
            if debug {
                eprintln!("[rir] invalid after {label}: {errors:?}");
            }
            false
        }
    }
}

/// Round trip through the Rakit mid-level IR. It fires for functions the
/// codegen IR<->RIR bridge supports; unsupported functions are left untouched.
pub struct RirPass<'a> {
    bodies: Option<&'a crate::passes::inliner::CguBodies>,
}

impl<'a> RirPass<'a> {
    pub fn new(bodies: &'a crate::passes::inliner::CguBodies) -> Self {
        Self { bodies: Some(bodies) }
    }
}

impl Pass for RirPass<'_> {
    fn name(&self) -> &'static str {
        "rir"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if !crate::tunables::rir_enabled(pctx.crate_name, pctx.user_crate) {
            return false;
        }
        let debug = std::env::var_os("RAKIT_RIR_DEBUG").is_some();
        let stats_mode = std::env::var_os("RAKIT_RIR_STATS").is_some();
        let dump_match = std::env::var("RAKIT_RIR_DUMP").ok();
        let dump = dump_match.as_ref().is_some_and(|m| {
            format!("{:?}", pctx.ctx.func.name).contains(m)
        });
        if dump {
            eprintln!("[rir] IN {:?}\n{}", pctx.ctx.func.name, pctx.ctx.func.display());
        }
        let Some(mut rir_func) = crate::rir::import(&pctx.ctx.func) else {
            if debug || stats_mode {
                eprintln!(
                    "[rir] skip {:?}: globals={} sized={} dyn={} stack_limit={}",
                    pctx.ctx.func.name,
                    pctx.ctx.func.global_values.len(),
                    pctx.ctx.func.sized_stack_slots.len(),
                    pctx.ctx.func.dynamic_stack_slots.len(),
                    pctx.ctx.func.stack_limit.is_some(),
                );
                if stats_mode {
                    eprintln!("[rir] skip reason: {}", crate::rir::unsupported_reason(&pctx.ctx.func));
                }
            }
            if debug {
                eprintln!("{}", pctx.ctx.func.display());
            }
            return false;
        };
        let simplify_changed = if std::env::var_os("RAKIT_NO_RIR_SIMPLIFY").is_some() {
            false
        } else {
            rakit_ir::simplify::run(&mut rir_func)
        };
        let opt_changed = if std::env::var_os("RAKIT_NO_RIR_OPT").is_some() {
            false
        } else {
            rakit_ir::optimize(&mut rir_func)
        };
        let mut changed = simplify_changed || opt_changed;
        if simplify_changed {
            if !verified(&rir_func, "simplify", debug) {
                return false;
            }
        }
        let rec_changed =
            crate::tunables::rir_rec_loop_enabled() && rakit_ir::rec_loop::run(&mut rir_func);
        if rec_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if rec_changed && !verified(&rir_func, "rec_loop", debug) {
            return false;
        }
        if debug && rec_changed {
            eprintln!("[rir] rec_loop fired");
        }
        let unroll_changed =
            crate::tunables::rir_unroll_enabled() && rakit_ir::unroll::run(&mut rir_func);
        if unroll_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if unroll_changed && !verified(&rir_func, "unroll", debug) {
            return false;
        }
        if debug && unroll_changed {
            eprintln!("[rir] unroll fired");
        }
        if debug && simplify_changed {
            eprintln!("[rir] simplify fired");
        }
        if let Some(bodies) = self.bodies {
            let inline_changed = crate::tunables::rir_inline_enabled()
                && rakit_ir::inliner::run(&mut rir_func, bodies.rir_bodies());
            if inline_changed {
                rakit_ir::optimize(&mut rir_func);
                changed = true;
            }
            if inline_changed && !verified(&rir_func, "inliner", debug) {
                return false;
            }
            if debug && inline_changed {
                eprintln!("[rir] inliner fired");
            }
        }
        let mem_changed = if std::env::var_os("RAKIT_NO_RIR_MEM").is_some() {
            false
        } else {
            rakit_ir::mem_forward::run(&mut rir_func)
        };
        if mem_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if mem_changed && !verified(&rir_func, "mem_forward", debug) {
            return false;
        }
        if debug && mem_changed {
            eprintln!("[rir] mem_forward fired");
        }
        let sroa_changed =
            crate::tunables::rir_sroa_enabled() && rakit_ir::sroa::run(&mut rir_func);
        if sroa_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if sroa_changed && !verified(&rir_func, "sroa", debug) {
            return false;
        }
        if debug && sroa_changed {
            eprintln!("[rir] sroa fired {:?}", pctx.ctx.func.name);
        }
        let div_changed = rakit_ir::div_test::run(&mut rir_func);
        if div_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if div_changed && !verified(&rir_func, "div_test", debug) {
            return false;
        }
        if debug && div_changed {
            eprintln!("[rir] div_test fired");
        }
        let ifconv_changed = rakit_ir::if_convert::run(&mut rir_func);
        if ifconv_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if ifconv_changed && !verified(&rir_func, "if_convert", debug) {
            return false;
        }
        if debug && ifconv_changed {
            eprintln!("[rir] if_convert fired");
        }
        let gvn_changed = if std::env::var_os("RAKIT_NO_RIR_GVN").is_some() {
            false
        } else {
            rakit_ir::gvn::run(&mut rir_func)
        };
        if gvn_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if gvn_changed && !verified(&rir_func, "gvn", debug) {
            return false;
        }
        if debug && gvn_changed {
            eprintln!("[rir] gvn fired");
        }
        let licm_changed = if std::env::var_os("RAKIT_NO_RIR_LICM").is_some() {
            false
        } else {
            rakit_ir::licm::run(&mut rir_func)
        };
        if licm_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if licm_changed && !verified(&rir_func, "licm", debug) {
            return false;
        }
        if debug && licm_changed {
            eprintln!("[rir] licm fired");
        }
        let sincos_changed =
            crate::tunables::rir_sincos_enabled() && rakit_ir::sincos::run(&mut rir_func);
        if sincos_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if sincos_changed && !verified(&rir_func, "sincos", debug) {
            return false;
        }
        if debug && sincos_changed {
            eprintln!("[rir] sincos fired");
        }
        if !verified(&rir_func, "pipeline", debug) {
            return false;
        }
        if crate::tunables::custom_backend_enabled() {
            let selector = rakit_ir::backend::RirSelector;
            match selector.select(&rir_func) {
                Ok(prealloc) if prealloc.is_fallback() => {
                    // The selector could not represent this function.  Keep the
                    // existing RIR/CLIF body; re-exporting a copied RIR body is
                    // not a production emitter and can lose bridge-only details.
                    if debug || stats_mode {
                        eprintln!("[rir] custom isel fallback");
                    }
                }
                Ok(prealloc) => {
                    let mut mach = prealloc.clone();
                    match rakit_ir::backend::LinearScan.allocate(&mut mach) {
                        Ok(stats) => match rakit_ir::backend::mach::to_rir(&prealloc) {
                            Ok(emitted) if rakit_ir::verify(&emitted).is_ok() => {
                                rir_func = emitted;
                                changed = true;
                                if debug || stats_mode {
                                    eprintln!(
                                        "[rir] custom backend emitted: {} vregs, {} regs, {} spills",
                                        stats.vregs, stats.registers, stats.spills
                                    );
                                }
                            }
                            Ok(_) => {
                                if debug {
                                    eprintln!("[rir] custom emitter produced invalid RIR");
                                }
                            }
                            Err(err) => {
                                if debug {
                                    eprintln!("[rir] custom emitter skip: {err}");
                                }
                            }
                        },
                        Err(err) => {
                            if debug {
                                eprintln!("[rir] custom regalloc skip: {err}");
                            }
                        }
                    }
                }
                Err(err) => {
                    if debug {
                        eprintln!("[rir] custom isel skip: {err}");
                    }
                }
            }
        }
        if !changed && std::env::var_os("RAKIT_RIR_FORCE").is_none() {
            if debug {
                eprintln!("[rir] no-change {:?}", pctx.ctx.func.name);
                eprintln!("{}", pctx.ctx.func.display());
            }
            return false;
        }
        if dump {
            eprintln!("[rir] RIR\n{rir_func:?}");
        }
        let sig = pctx.ctx.func.signature.clone();
        let Some(mut new_func) = crate::rir::export(&rir_func, &sig) else {
            return false;
        };
        let flags = rakit_codegen::settings::Flags::new(rakit_codegen::settings::builder());
        let verified = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rakit_codegen::verify_function(&new_func, &flags)
        }));
        match verified {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                if debug || stats_mode {
                    eprintln!("[rir] export verify failed in {:?}: {err:?}", pctx.ctx.func.name);
                }
                return false;
            }
            Err(_) => {
                if debug || stats_mode {
                    eprintln!("[rir] export verifier panicked in {:?}", pctx.ctx.func.name);
                }
                return false;
            }
        }
        new_func.name = pctx.ctx.func.name.clone();
        if dump {
            eprintln!("[rir] OUT {:?}\n{}", pctx.ctx.func.name, new_func.display());
        }
        if debug || stats_mode {
            let what = if changed { "optimized" } else { "roundtrip" };
            eprintln!("[rir] {} {:?}", what, pctx.ctx.func.name);
        }
        pctx.ctx.func = new_func;
        true
    }
}

pub struct RecLoopPass;

impl Pass for RecLoopPass {
    fn name(&self) -> &'static str {
        "rec_loop"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if std::env::var_os("RAKIT_NO_REC_LOOP").is_none() {
            rec_loop::run(&mut pctx.ctx.func);
        }
        true
    }
}

pub struct StoreForwardPass;

impl Pass for StoreForwardPass {
    fn name(&self) -> &'static str {
        "store_forward"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        let endianness = pctx.module.isa().endianness();
        passtime::time(passtime::STORE_FORWARD, || {
            store_forward::run(&mut pctx.ctx.func, endianness)
        });
        true
    }
}

pub struct DivTestPass;

impl Pass for DivTestPass {
    fn name(&self) -> &'static str {
        "div_test"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        passtime::time(passtime::DIV_TEST, || div_test::run(&mut pctx.ctx.func));
        true
    }
}

pub struct IfConvertPass;

impl Pass for IfConvertPass {
    fn name(&self) -> &'static str {
        "if_convert"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        passtime::time(passtime::IF_CONVERT, || if_convert::run(&mut pctx.ctx.func));
        true
    }
}

pub struct UnrollPass;

impl Pass for UnrollPass {
    fn name(&self) -> &'static str {
        "unroll"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        passtime::time(passtime::UNROLL, || unroll::run(&mut pctx.ctx.func));
        true
    }
}

pub struct SincosPass;

impl Pass for SincosPass {
    fn name(&self) -> &'static str {
        "sincos"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if std::env::var_os("RAKIT_NO_SINCOS").is_none() {
            sincos::run(&mut *pctx.module, &mut pctx.ctx.func);
        }
        true
    }
}
