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
use rustc_data_structures::fx::{FxHashMap, FxHashSet};

/// Largest callee (in CLIF instructions) we are willing to inline.
pub(crate) const MAX_CALLEE_INSTS: usize = 64;

/// Total number of callee instructions we are willing to add to a single caller.
const INLINE_BUDGET: usize = 512;

/// `CG_CLIF_INLINE_MAX` overrides [`MAX_CALLEE_INSTS`] (0 disables inlining); used for bisecting.
fn max_callee_insts() -> usize {
    match std::env::var("CG_CLIF_INLINE_MAX") {
        Ok(v) => v.parse().unwrap_or(MAX_CALLEE_INSTS),
        Err(_) => MAX_CALLEE_INSTS,
    }
}

pub(crate) struct CguInliner<'a> {
    bodies: &'a FxHashMap<u32, ir::Function>,
    self_id: u32,
    inlined: FxHashSet<u32>,
    budget: usize,
}

impl<'a> CguInliner<'a> {
    pub(crate) fn new(bodies: &'a FxHashMap<u32, ir::Function>, self_id: u32) -> Self {
        CguInliner { bodies, self_id, inlined: FxHashSet::default(), budget: INLINE_BUDGET }
    }
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
        // Never inline a function into itself, and inline each callee at most once per caller so
        // that mutual recursion terminates and code size stays bounded.
        if id == self.self_id || self.inlined.contains(&id) {
            return InlineCommand::KeepCall;
        }
        let bodies = self.bodies;
        let Some(body) = bodies.get(&id) else {
            return InlineCommand::KeepCall;
        };
        let size = body.dfg.num_insts();
        if size > max_callee_insts() || size > self.budget {
            return InlineCommand::KeepCall;
        }
        // Cranelift's inliner clones the callee's `GlobalValueData` verbatim but does not remap
        // the `ExternalName` indices inside `GlobalValueData::Symbol` to the caller's namespace,
        // so a callee that references a symbol (a const/static/thread-local) must not be inlined.
        if !body.global_values.is_empty() {
            return InlineCommand::KeepCall;
        }
        self.inlined.insert(id);
        self.budget -= size;
        InlineCommand::Inline { callee: Cow::Borrowed(body), visit_callee: true }
    }
}
