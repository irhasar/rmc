//! First-party optimization passes and their `Pass` adapters.

pub mod div_test;
pub mod if_convert;
pub mod inliner;
pub mod rec_loop;
pub mod rotate;
pub mod sincos;
pub mod store_forward;
pub mod unroll;
pub mod vectorize;

use crate::pass::{Pass, PassCtx};
use crate::passtime;

use rakit_codegen::ir::{self, Opcode};

/// Cheap necessary-condition prefilter: does any instruction use `want`? An
/// opcode-only walk is far cheaper than the per-pass setup (use-count maps,
/// CFG/dominator construction) the pass would otherwise run before finding
/// nothing, and the vast majority of functions do not contain the opcode a
/// given pass looks for.
fn has_opcode(func: &ir::Function, want: Opcode) -> bool {
    func.layout
        .blocks()
        .any(|b| func.layout.block_insts(b).any(|i| func.dfg.insts[i].opcode() == want))
}

pub struct RecLoopPass;

impl Pass for RecLoopPass {
    fn name(&self) -> &'static str {
        "rec_loop"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if std::env::var_os("RAKIT_NO_REC_LOOP").is_some() {
            return false;
        }
        // `rec_loop::try_match` already rejects non-recursive functions at the
        // entry terminator (O(1)); a full opcode pre-scan here would be *slower*
        // for call-free straight-line functions, so time it directly.
        passtime::time(passtime::REC_LOOP, || rec_loop::run(&mut pctx.ctx.func))
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
        // Respect the disable switch before doing any work: a disabled pass must
        // cost nothing, not even the opcode pre-scan.
        if !crate::tunables::div_test_enabled() {
            return false;
        }
        // The pre-scan is inside the timer so the reported time includes it, and
        // the closure's result is the pass result: `div_test::run` reports
        // whether it rewrote anything, so a function with no candidate or a
        // rejected function both yield `false`.
        passtime::time(passtime::DIV_TEST, || {
            if has_opcode(&pctx.ctx.func, Opcode::Urem) {
                div_test::run(&mut pctx.ctx.func)
            } else {
                false
            }
        })
    }
}

pub struct IfConvertPass;

impl Pass for IfConvertPass {
    fn name(&self) -> &'static str {
        "if_convert"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        if !crate::tunables::if_convert_enabled() {
            return false;
        }
        // A two-way diamond needs a head, two arms and a join (>=3 blocks) and a
        // `brif` head; skip otherwise. The block-count check and the opcode scan
        // are inside the timer so the reported time includes them, and the
        // closure's result is the pass result (`if_convert::run` reports whether
        // it converted a diamond).
        passtime::time(passtime::IF_CONVERT, || {
            if pctx.ctx.func.layout.blocks().nth(2).is_some()
                && has_opcode(&pctx.ctx.func, Opcode::Brif)
            {
                if_convert::run(&mut pctx.ctx.func, pctx.analyses)
            } else {
                false
            }
        })
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
        if std::env::var_os("RAKIT_NO_SINCOS").is_some() {
            return false;
        }
        // `sincos::run` already does one cheap walk to find sin/cos calls; a
        // separate Call pre-scan would add a walk, so time it directly.
        passtime::time(passtime::SINCOS, || {
            sincos::run(&mut *pctx.module, &mut pctx.ctx.func, pctx.analyses)
        })
    }
}

pub struct VectorizePass;

impl Pass for VectorizePass {
    fn name(&self) -> &'static str {
        "vectorize"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        vectorize::run(&mut pctx.ctx.func)
    }
}
