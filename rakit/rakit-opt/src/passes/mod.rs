//! First-party optimization passes and their `Pass` adapters.

pub mod div_test;
pub mod if_convert;
pub mod inliner;
pub mod rec_loop;
pub mod sincos;
pub mod store_forward;
pub mod unroll;

use crate::passtime;
use crate::pass::{Pass, PassCtx};

/// Optional round trip through the Rakit mid-level IR. It only fires for
/// functions the CLIF<->RIR bridge supports (scalar, no memory/calls) and only
/// when `RAKIT_RIR` is set, so it cannot affect a normal build.
pub struct RirPass;

impl Pass for RirPass {
    fn name(&self) -> &'static str {
        "rir"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if !crate::tunables::rir_enabled() {
            return false;
        }
        let debug = std::env::var_os("RAKIT_RIR_DEBUG").is_some();
        let Some(mut rir_func) = crate::rir::import(&pctx.ctx.func) else {
            if debug {
                eprintln!(
                    "[rir] skip {:?}: globals={} sized={} dyn={} stack_limit={}",
                    pctx.ctx.func.name,
                    pctx.ctx.func.global_values.len(),
                    pctx.ctx.func.sized_stack_slots.len(),
                    pctx.ctx.func.dynamic_stack_slots.len(),
                    pctx.ctx.func.stack_limit.is_some(),
                );
                eprintln!("{}", pctx.ctx.func.display());
            }
            return false;
        };
        let simplify_changed = rakit_ir::simplify::run(&mut rir_func);
        let opt_changed = rakit_ir::optimize(&mut rir_func);
        let mut changed = simplify_changed || opt_changed;
        if debug && simplify_changed {
            eprintln!("[rir] simplify fired");
        }
        let div_changed = rakit_ir::div_test::run(&mut rir_func);
        if div_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if debug && div_changed {
            eprintln!("[rir] div_test fired");
        }
        let ifconv_changed = rakit_ir::if_convert::run(&mut rir_func);
        if ifconv_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if debug && ifconv_changed {
            eprintln!("[rir] if_convert fired");
        }
        let gvn_changed = rakit_ir::gvn::run(&mut rir_func);
        if gvn_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if debug && gvn_changed {
            eprintln!("[rir] gvn fired");
        }
        let licm_changed = rakit_ir::licm::run(&mut rir_func);
        if licm_changed {
            rakit_ir::optimize(&mut rir_func);
            changed = true;
        }
        if debug && licm_changed {
            eprintln!("[rir] licm fired");
        }
        if !changed && std::env::var_os("RAKIT_RIR_FORCE").is_none() {
            if debug {
                eprintln!("[rir] no-change {:?}", pctx.ctx.func.name);
                eprintln!("{}", pctx.ctx.func.display());
            }
            return false;
        }
        let sig = pctx.ctx.func.signature.clone();
        let Some(mut new_func) = crate::rir::export(&rir_func, &sig) else {
            return false;
        };
        new_func.name = pctx.ctx.func.name.clone();
        new_func.params = pctx.ctx.func.params.clone();
        if std::env::var_os("RAKIT_RIR_DEBUG").is_some() {
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
        rec_loop::run(&mut pctx.ctx.func);
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
        passtime::time(passtime::IF_CONVERT, || {
            if_convert::run(&mut pctx.ctx.func)
        });
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
        sincos::run(&mut *pctx.module, &mut pctx.ctx.func);
        true
    }
}
