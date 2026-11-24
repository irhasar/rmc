//! First-party optimization passes and their `Pass` adapters.

pub mod div_test;
pub mod if_convert;
pub mod inliner;
pub mod rec_loop;
pub mod rotate;
pub mod sincos;
pub mod store_forward;
pub mod unroll;

use crate::pass::{Pass, PassCtx};
use crate::passtime;

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
            store_forward::run(&mut pctx.ctx.func, endianness, pctx.analyses)
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
        // A two-way diamond needs a head, two arms and a join, i.e. at least
        // three blocks. Below that the pass can only scan and find nothing, so
        // skip it: on a crate with thousands of small functions this is most of
        // the compile cost.
        if pctx.ctx.func.layout.blocks().nth(2).is_none() {
            return false;
        }
        passtime::time(passtime::IF_CONVERT, || if_convert::run(&mut pctx.ctx.func, pctx.analyses));
        true
    }
}

pub struct UnrollPass;

impl Pass for UnrollPass {
    fn name(&self) -> &'static str {
        "unroll"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        // `unroll::run` also does the loop-independent `narrow_uint_fcvt` rewrite,
        // which applies to any function, so it must not be skipped on small
        // functions.
        passtime::time(passtime::UNROLL, || unroll::run(&mut pctx.ctx.func, pctx.analyses));
        true
    }
}

pub struct RotatePass;

impl Pass for RotatePass {
    fn name(&self) -> &'static str {
        "rotate"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if pctx.ctx.func.layout.blocks().nth(1).is_none() {
            return false;
        }
        passtime::time(passtime::ROTATE, || rotate::run(&mut pctx.ctx.func));
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
            sincos::run(&mut *pctx.module, &mut pctx.ctx.func, pctx.analyses);
        }
        true
    }
}
