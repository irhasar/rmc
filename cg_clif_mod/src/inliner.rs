//! A conservative function inliner operating on CLIF.
//!
//! Cranelift's [`cranelift_codegen::Context::inline`] provides the mechanics of
//! inlining but deliberately leaves the heuristics and the callee bodies to the
//! embedder. cg_clif emits one CLIF function per mono item and, without this
//! pass, never inlines calls, so even tiny helpers such as a three-`add` chain
//! stay out of line. This module supplies the missing heuristic: a direct call
//! is inlined when the callee body is available in the same codegen unit and is
//! small enough, bounded by a budget so code size cannot blow up and recursion
//! cannot loop forever.

use std::borrow::Cow;

use cranelift_codegen::ir;
use cranelift_codegen::inline::{Inline, InlineCommand};
use rustc_data_structures::fx::FxHashMap;

/// Largest callee (in CLIF instructions) we are willing to inline.
pub(crate) const MAX_CALLEE_INSTS: usize = 64;

/// Total number of callee instructions we are willing to add to a single caller.
const INLINE_BUDGET: usize = 512;

/// Lower bound on the growth budget, so a tiny wrapper can still absorb callees.
const MIN_GROWTH: usize = 64;

/// Maximum number of times a single callee may be inlined into one caller.
pub(crate) const MAX_CALLEE_USES: usize = 3;

/// Default growth budget, as a percentage of the caller's instruction count.
pub(crate) const GROWTH_PCT: usize = 50;

/// `CG_CLIF_INLINE_USES` overrides [`MAX_CALLEE_USES`] (0 disables inlining); used for bisecting.
fn max_callee_uses() -> usize {
    crate::tunables::inline_max_uses()
}

/// `CG_CLIF_INLINE_MAX` overrides [`MAX_CALLEE_INSTS`] (0 disables inlining); used for bisecting.
pub(crate) fn max_callee_insts() -> usize {
    crate::tunables::inline_max_insts()
}

pub(crate) struct CguInliner<'a> {
    bodies: &'a FxHashMap<u32, ir::Function>,
    self_id: u32,
    inlined: FxHashMap<u32, usize>,
    budget: usize,
    max_uses: usize,
    max_insts: usize,
}

impl<'a> CguInliner<'a> {
    pub(crate) fn new(
        bodies: &'a FxHashMap<u32, ir::Function>,
        self_id: u32,
        caller: &ir::Function,
    ) -> Self {
        CguInliner {
            bodies,
            self_id,
            inlined: FxHashMap::default(),
            // Inlining a callee costs Cranelift time proportional to the code it
            // adds, on *every* later pass. Bound the growth to a fraction of the
            // caller so a large function cannot balloon: the runtime win comes
            // from small helpers in hot code, not from doubling big bodies.
            budget: inline_budget(caller.dfg.num_insts()),
            // Read the knobs once: `inline` runs per call site and a `std::env`
            // lookup on each is pure overhead.
            max_uses: max_callee_uses(),
            max_insts: max_callee_insts(),
        }
    }
}

/// How many CLIF instructions the inliner may add to a caller of `caller_insts`
/// instructions. Never below [`MIN_GROWTH`] so tiny wrappers can still absorb
/// their callees, never above [`INLINE_BUDGET`] so compile time stays bounded.
fn inline_budget(caller_insts: usize) -> usize {
    let cap = caller_insts.saturating_mul(crate::tunables::inline_growth_pct()) / 100;
    cap.clamp(MIN_GROWTH, INLINE_BUDGET)
}

/// Recover the [`FuncId`](cranelift_module::FuncId) behind a direct call.
///
/// `Module::declare_func_in_func` encodes the callee as a user external name
/// whose index is the `FuncId`, so this reverses that encoding.
fn callee_func_id(func: &ir::Function, func_ref: ir::FuncRef) -> Option<u32> {
    match func.dfg.ext_funcs[func_ref].name {
        ir::ExternalName::User(reff) => Some(func.params.user_named_funcs()[reff].index),
        _ => None,
    }
}

impl Inline for CguInliner<'_> {
    fn inline(
        &mut self,
        caller: &ir::Function,
        _call_inst: ir::Inst,
        _call_opcode: ir::Opcode,
        callee: ir::FuncRef,
        _call_args: &[ir::Value],
    ) -> InlineCommand<'_> {
        let Some(id) = callee_func_id(caller, callee) else {
            return InlineCommand::KeepCall;
        };
        // Never inline a function into itself. Inline each callee up to
        // `max_callee_uses()` times per caller so that hot small callees called
        // several times are still inlined, while the call-graph budget keeps
        // mutual recursion and code-size growth bounded.
        if id == self.self_id || self.inlined.get(&id).is_some_and(|&n| n >= self.max_uses) {
            return InlineCommand::KeepCall;
        }
        let bodies = self.bodies;
        let Some(body) = bodies.get(&id) else {
            return InlineCommand::KeepCall;
        };
        let size = body.dfg.num_insts();
        if size > self.max_insts || size > self.budget {
            return InlineCommand::KeepCall;
        }
        self.inlined.insert(id, self.inlined.get(&id).copied().unwrap_or(0) + 1);
        self.budget -= size;
        // `visit_callee: true` lets the walk descend into the freshly inlined
        // body so a callee's own calls are inlined too. This is what lets a
        // wrapper chain collapse; the budget bounds the growth.
        InlineCommand::Inline { callee: Cow::Borrowed(body), visit_callee: true }
    }
}
