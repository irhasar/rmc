//! A conservative function inliner operating on RAKIT.
//!
//! Rakit's [`rakit_codegen::Context::inline`] provides the mechanics of
//! inlining but deliberately leaves the heuristics and the callee bodies to the
//! embedder. rakit emits one RAKIT function per mono item and, without this
//! pass, never inlines calls, so even tiny helpers such as a three-`add` chain
//! stay out of line. This module supplies the missing heuristic: a direct call
//! is inlined when the callee body is available in the same codegen unit and is
//! small enough, bounded by a budget so code size cannot blow up and recursion
//! cannot loop forever.

use std::borrow::Cow;
use std::collections::HashMap as FxHashMap;
use std::sync::OnceLock;

use rakit_codegen::inline::{Inline, InlineCommand};
use rakit_codegen::ir;

/// Largest callee (in RAKIT instructions) we are willing to inline.
pub const MAX_CALLEE_INSTS: usize = 128;

/// Largest *leaf* callee (no calls of its own) we are willing to inline.
///
/// Leaf bodies are the profitable case: inlining one lets the caller's later
/// passes and register allocator see the arithmetic directly instead of a call
/// boundary, which is what turns a per-iteration helper call into straight-line
/// code. They cannot pull in further callees, so a larger cap here does not
/// cascade the way it would for a wrapper.
///
/// The iterator chain a `collect` lowers to is the motivating case: `extend_trusted`
/// hands the element loop to `fold`, which calls `Range::spec_next` and the
/// `map_fold` closure once per element. Those bodies are a few hundred
/// instructions together, above the old 192 cap, so the loop kept two real calls
/// per element along with the spill and reload of the iterator state around each
/// one. Raising the cap lets the loop become straight-line code.
pub const MAX_LEAF_INSTS: usize = 384;

/// Total number of callee instructions we are willing to add to a single caller.
const INLINE_BUDGET: usize = 512;

/// A callee inlined at least this many times into one caller is "hot" there.
/// Further inlines may draw on [`HOT_BUDGET`] even after the ordinary growth
/// budget is spent, so a sorting network's per-element swap calls collapse
/// instead of stopping after the first handful.
const HOT_CALLS: usize = 1;
/// Extra instruction budget a caller may spend only on hot callees.
const HOT_BUDGET: usize = 1024;

/// `RAKIT_NO_HOT_INLINE` disables the hot-callee budget (bisecting knob).
fn hot_calls() -> usize {
    use std::sync::OnceLock;
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| {
        if std::env::var_os("RAKIT_NO_HOT_INLINE").is_some() { usize::MAX } else { HOT_CALLS }
    })
}

/// Lower bound on the growth budget, so a tiny wrapper can still absorb callees.
const MIN_GROWTH: usize = 64;

/// Maximum number of times a single callee may be inlined into one caller.
pub const MAX_CALLEE_USES: usize = 5;

/// Trivial leaves are exempt from [`MAX_CALLEE_USES`].
///
/// A `#[inline]` one-liner such as `<f32 as Mul>::mul` is emitted by rakit as
/// an out-of-line function, and a hot loop may call it dozens of times. Capping
/// those at three uses leaves the rest as real calls, which costs far more than
/// the few instructions the inline adds: on a blur kernel it was the difference
/// between 22.3 s and 12.8 s of runtime. The total growth is still bounded by
/// `leaf_budget`, so the exemption cannot blow up code size.
///
/// The cap is 16 rather than 8 because the iterator adapter bodies a
/// bounds-checked loop lowers to (`__iterator_get_unchecked`) are 12-13 RAKIT
/// instructions, just above the smaller value; inlining them removed the last
/// calls from the hot blur loops and was worth another 16%.
pub const TRIVIAL_LEAF_INSTS: usize = 16;

/// `RAKIT_INLINE_TRIVIAL_MAX` overrides [`TRIVIAL_LEAF_INSTS`]; used to bisect.
pub fn trivial_leaf_insts() -> usize {
    crate::tunables::inline_trivial_max_insts()
}

/// Default growth budget, as a percentage of the caller's instruction count.
pub const GROWTH_PCT: usize = 150;

/// `RAKIT_INLINE_USES` overrides [`MAX_CALLEE_USES`] (0 disables inlining); used for bisecting.
fn max_callee_uses() -> usize {
    crate::tunables::inline_max_uses()
}

/// `RAKIT_INLINE_MAX` overrides [`MAX_CALLEE_INSTS`] (0 disables inlining); used for bisecting.
pub fn max_callee_insts() -> usize {
    crate::tunables::inline_max_insts()
}

/// `RAKIT_INLINE_LEAF_MAX` overrides [`MAX_LEAF_INSTS`].
pub fn max_leaf_insts() -> usize {
    crate::tunables::inline_leaf_max_insts()
}

/// Whether `body` contains no call of its own (a leaf).
///
/// A leaf callee has no callees to pull in, so inlining it cannot cascade; this
/// is what makes a larger cap safe for leaves while non-leaves keep the small one.
///
/// Blocks that end in a `trap` are panic/abort paths (e.g. a failed bounds
/// check). The calls they contain only format the panic message and never run on
/// the success path, so they do not stop a body from being treated as a leaf.
fn is_leaf(body: &ir::Function) -> bool {
    body.layout.blocks().all(|block| {
        let traps = body
            .layout
            .last_inst(block)
            .is_some_and(|term| body.dfg.insts[term].opcode() == ir::Opcode::Trap);
        traps
            || body.layout.block_insts(block).all(|inst| {
                !matches!(
                    body.dfg.insts[inst].opcode(),
                    ir::Opcode::Call | ir::Opcode::CallIndirect
                )
            })
    })
}

pub struct CguInliner<'a> {
    bodies: &'a CguBodies,
    self_id: u32,
    inlined: FxHashMap<u32, usize>,
    budget: usize,
    leaf_budget: usize,
    hot_budget: usize,
    max_uses: usize,
    max_insts: usize,
    max_leaf_insts: usize,
}

/// The callee bodies of one codegen unit, with the facts the inliner needs about
/// each one computed once at collection time rather than per call site.
///
/// `is_leaf` walks every instruction of a body, and the inliner asks about the
/// same callee from every call site that names it. On a large crate that turned
/// into the same scan repeated hundreds of times per function; measuring showed
/// the scan, not the inlining itself, as the dominant cost of the pass.
#[derive(Default)]
pub struct CguBodies {
    funcs: FxHashMap<u32, CguBody>,
    rir: OnceLock<Vec<rakit_ir::Function>>,
    rir_eligible: usize,
}

struct CguBody {
    func: ir::Function,
    is_leaf: bool,
}

impl CguBodies {
    /// Collect every final function of the codegen unit no larger than `max_insts`.
    ///
    /// Only small, final (non-interposable) callees are kept, to bound memory and
    /// to avoid changing the semantics of separately compiled symbols.
    pub fn collect<'f>(
        funcs: impl Iterator<Item = (u32, &'f ir::Function)>,
        max_insts: usize,
    ) -> Self {
        let mut out = FxHashMap::default();
        for (id, func) in funcs {
            if func.dfg.num_insts() > max_insts {
                continue;
            }
            out.insert(id, CguBody { func: func.clone(), is_leaf: is_leaf(func) });
        }
        CguBodies { funcs: out, rir: OnceLock::new(), rir_eligible: 0 }
    }

    /// Import the collected bodies once for the RIR inliner.  A codegen unit
    /// is compiled many times (once per function), so the conversion must be
    /// shared rather than repeated per caller.
    pub fn rir_bodies(&self) -> &[rakit_ir::Function] {
        self.rir.get_or_init(|| {
            self.funcs
                .values()
                .filter_map(|body| {
                    let mut f = crate::rir::import(&body.func)?;
                    rakit_ir::simplify::run(&mut f);
                    Some(f)
                })
                .collect()
        })
    }

    pub fn is_empty(&self) -> bool {
        self.funcs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.funcs.len()
    }

    /// Number of functions in the whole CGU that the RIR bridge supports. This
    /// is independent of the inliner's size-filtered cache.
    pub fn rir_eligible(&self) -> usize {
        self.rir_eligible
    }

    pub fn set_rir_eligible(&mut self, count: usize) {
        self.rir_eligible = count;
    }
}

impl<'a> CguInliner<'a> {
    pub fn new(bodies: &'a CguBodies, self_id: u32, caller: &ir::Function) -> Self {
        let caller_insts = caller.dfg.num_insts();
        // `RAKIT_INLINE_MAX=0` keeps its documented meaning: inlining is off,
        // including the leaf tier, so the knob can still bisect it away.
        let (max_insts, max_leaf_insts) =
            if max_callee_insts() == 0 { (0, 0) } else { (max_callee_insts(), max_leaf_insts()) };
        CguInliner {
            bodies,
            self_id,
            inlined: FxHashMap::default(),
            // Inlining a callee costs Rakit time proportional to the code it
            // adds, on *every* later pass. Bound the growth to a fraction of the
            // caller so a large function cannot balloon: the runtime win comes
            // from small helpers in hot code, not from doubling big bodies.
            budget: inline_budget(caller_insts),
            // Leaves get their own allowance, floored at one leaf-sized callee.
            // A hot loop body is often small, so a growth-only budget would
            // reject the very helper the loop calls on every iteration. It is
            // *added* to the caller's budget rather than replacing it so a big
            // caller does not lose headroom for its ordinary callees.
            leaf_budget: inline_budget(caller_insts).saturating_add(max_leaf_insts),
            hot_budget: HOT_BUDGET,
            // Read the knobs once: `inline` runs per call site and a `std::env`
            // lookup on each is pure overhead.
            max_uses: max_callee_uses(),
            max_insts,
            max_leaf_insts,
        }
    }
}

/// How many RAKIT instructions the inliner may add to a caller of `caller_insts`
/// instructions. Never below [`MIN_GROWTH`] so tiny wrappers can still absorb
/// their callees, never above [`INLINE_BUDGET`] so compile time stays bounded.
fn inline_budget(caller_insts: usize) -> usize {
    let cap = caller_insts.saturating_mul(crate::tunables::inline_growth_pct()) / 100;
    cap.clamp(MIN_GROWTH, INLINE_BUDGET)
}

/// Recover the [`FuncId`](rakit_module::FuncId) behind a direct call.
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
        // mutual recursion and code-size growth bounded. Trivial leaves are
        // exempt: they are single operations, and a remaining call site costs
        // more than the inline. Recursion is still impossible because a leaf
        // has no calls of its own.
        let trivial = id != self.self_id
            && crate::tunables::inline_trivial_leaf()
            && self
                .bodies
                .funcs
                .get(&id)
                .is_some_and(|e| e.is_leaf && e.func.dfg.num_insts() <= trivial_leaf_insts());
        if id == self.self_id
            || (!trivial && self.inlined.get(&id).is_some_and(|&n| n >= self.max_uses))
        {
            return InlineCommand::KeepCall;
        }
        let bodies = &self.bodies.funcs;
        let Some(entry) = bodies.get(&id) else {
            return InlineCommand::KeepCall;
        };
        let body = &entry.func;
        let size = body.dfg.num_insts();
        // Leaves get the larger cap and their own budget; a leaf cannot pull in
        // further callees, so the extra instructions are pure straight-line code
        // rather than the start of a call-graph expansion. Whether a body is a
        // leaf is decided once, when the CGU's bodies are collected.
        let leaf = entry.is_leaf;
        let (cap, budget) = if leaf {
            (self.max_leaf_insts, self.leaf_budget)
        } else {
            (self.max_insts, self.budget)
        };
        if size > cap {
            return InlineCommand::KeepCall;
        }
        // A callee already inlined a few times into this caller is hot here: if
        // the ordinary budget cannot cover another copy, let it draw on the hot
        // budget instead. This collapses a sorting network's many small swap
        // calls instead of stopping once the growth budget is spent.
        let hot = !trivial && self.inlined.get(&id).copied().unwrap_or(0) >= hot_calls();
        let use_hot = size > budget && hot && size <= self.hot_budget;
        if size > budget && !use_hot {
            return InlineCommand::KeepCall;
        }
        // A trivial leaf is a single operation, so it must not compete for a
        // budget meant to bound real code growth. Charging it is what left the
        // last `Add::add` calls in a hot blur loop once the caller had already
        // absorbed its larger callees.
        if !trivial {
            if use_hot {
                self.hot_budget -= size;
            } else if leaf {
                self.leaf_budget -= size;
            } else {
                self.budget -= size;
            }
        }
        self.inlined.insert(id, self.inlined.get(&id).copied().unwrap_or(0) + 1);
        // `visit_callee: true` lets the walk descend into the freshly inlined
        // body so a callee's own calls are inlined too. This is what lets a
        // wrapper chain collapse; the budget bounds the growth.
        InlineCommand::Inline { callee: Cow::Borrowed(body), visit_callee: true }
    }
}
