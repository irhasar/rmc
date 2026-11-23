//! Process-wide tunables, read from the environment once.
//!
//! The RAKIT passes run once per function, so a `std::env` lookup inside a pass
//! is paid thousands of times per crate. `std::env::var` locks a global map on
//! every call, which showed up as a measurable share of the pass cost. Every
//! knob is read exactly once here, on first use, and cached for the process.

use std::sync::OnceLock;

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn flag(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

/// Whether the store-forwarding pass runs at all (`RAKIT_NO_STORE_FWD` off).
pub fn store_forward_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_STORE_FWD"))
}

/// Whether the RIR round-trip pass runs. It is on for every crate, including
/// the standard library; unsupported functions are skipped by the bridge and
/// any function whose RIR fails verification is left untouched.
/// `RAKIT_NO_RIR` disables it for bisecting.
pub fn rir_enabled(_crate_name: &str, _user_crate: bool) -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_RIR"))
}

/// Whether the Phase 3 custom ISel/regalloc runs after RIR passes
/// (`RAKIT_CUSTOM_BACKEND` on). It validates the RIR through the machine-IR
/// backend but does not replace codegen IR emission.
pub fn custom_backend_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_CUSTOM_BACKEND"))
}

/// Whether RIR SROA runs.  The default tier promotes only whole scalar slots
/// in straight-line leaf functions; `RAKIT_NO_RIR_SROA=1` disables it for
/// bisecting and `RAKIT_RIR_SROA=1` remains accepted for compatibility.
pub fn rir_sroa_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_RIR_SROA"))
}

/// Whether RIR `rec_loop` runs (`RAKIT_NO_RIR_REC_LOOP` off).
pub fn rir_rec_loop_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_RIR_REC_LOOP"))
}

/// Whether the RIR inliner runs (`RAKIT_NO_RIR_INLINE` off).
pub fn rir_inline_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_RIR_INLINE"))
}

/// Whether RIR loop unrolling runs. Off by default: the codegen IR unroller
/// implements affine reassociation, while the RIR unroller only duplicates
/// the body and regresses loop-carried chains. Enable with `RAKIT_RIR_UNROLL=1`.
pub fn rir_unroll_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_RIR_UNROLL"))
}

/// Whether RIR `sincos` fusion runs (`RAKIT_NO_RIR_SINCOS` off).
pub fn rir_sincos_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_RIR_SINCOS"))
}

/// Whether the divisibility-test pass runs at all (`RAKIT_NO_DIVTEST` off).
pub fn div_test_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_DIVTEST"))
}

/// Whether the if-conversion pass runs at all (`RAKIT_NO_IFCONV` off).
pub fn if_convert_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_IFCONV"))
}

/// Per-function store-forwarding timing output (`RAKIT_SF_TIME`).
pub fn store_forward_time() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_SF_TIME"))
}

/// Substring of function names the store-forwarding pass skips (`RAKIT_SF_SKIP`).
pub fn store_forward_skip() -> Option<&'static str> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| var("RAKIT_SF_SKIP")).as_deref()
}

/// Whether the cross-block dataflow of store forwarding is enabled.
pub fn store_forward_cross_block() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_SF_NO_CBE"))
}

/// Whether `Rvalue::Repeat` may lower to a memset (`RAKIT_NO_REPEAT_MEMSET`).
pub fn repeat_memset() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_REPEAT_MEMSET"))
}

/// Instruction-count budget above which the cross-block dataflow is skipped.
pub fn store_forward_cbe_max() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_SF_CBE_MAX") {
        Some(v) => v.parse().unwrap_or(usize::MAX),
        None => crate::passes::store_forward::CBE_MAX_INSTS,
    })
}

/// Largest callee (in RAKIT instructions) the inliner accepts (`RAKIT_INLINE_MAX`).
pub fn inline_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_INLINE_MAX") {
        Some(v) => v.parse().unwrap_or(crate::passes::inliner::MAX_CALLEE_INSTS),
        None => crate::passes::inliner::MAX_CALLEE_INSTS,
    })
}

/// Largest *leaf* callee (no calls of its own) the inliner accepts
/// (`RAKIT_INLINE_LEAF_MAX`).
///
/// A leaf body is pure computation: once inlined, it exposes its arithmetic to
/// the caller's passes and lets the register allocator keep values in registers
/// across the former call boundary, which is where the runtime win comes from.
/// Non-leaf bodies mainly move a call around and can still be duplicated
/// cheaply, so they keep the smaller [`inline_max_insts`] cap.
pub fn inline_leaf_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_INLINE_LEAF_MAX") {
        Some(v) => v.parse().unwrap_or(crate::passes::inliner::MAX_LEAF_INSTS),
        None => crate::passes::inliner::MAX_LEAF_INSTS,
    })
}

/// Maximum times one callee may be inlined per caller (`RAKIT_INLINE_USES`).
pub fn inline_max_uses() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_INLINE_USES") {
        Some(v) => v.parse().unwrap_or(crate::passes::inliner::MAX_CALLEE_USES),
        None => crate::passes::inliner::MAX_CALLEE_USES,
    })
}

/// Whether trivial leaves bypass the per-caller use cap (`RAKIT_INLINE_NO_TRIVIAL` off).
pub fn inline_trivial_leaf() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_INLINE_NO_TRIVIAL"))
}

/// Instruction cap for the trivial-leaf exemption (`RAKIT_INLINE_TRIVIAL_MAX`).
pub fn inline_trivial_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_INLINE_TRIVIAL_MAX") {
        Some(v) => v.parse().unwrap_or(crate::passes::inliner::TRIVIAL_LEAF_INSTS),
        None => crate::passes::inliner::TRIVIAL_LEAF_INSTS,
    })
}

/// Loop unroll factor (`RAKIT_UNROLL`).
pub fn unroll_factor() -> u64 {
    static V: OnceLock<u64> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_UNROLL") {
        Some(v) => v.parse().unwrap_or(crate::passes::unroll::UNROLL_FACTOR),
        None => crate::passes::unroll::UNROLL_FACTOR,
    })
}

/// Partial-reassociation factor for loops whose other state is a reduction or a
/// recomputable atom (`RAKIT_PARTIAL`).
pub fn partial_factor() -> u64 {
    static V: OnceLock<u64> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_PARTIAL") {
        Some(v) => v.parse().unwrap_or(crate::passes::unroll::PARTIAL_FACTOR),
        None => crate::passes::unroll::PARTIAL_FACTOR,
    })
}

/// Whether the unroller may model a bitwise op (`i | k`) as a recomputable atom
/// (`RAKIT_NO_ATOM` disables it for bisecting).
pub fn atom_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_ATOM"))
}

/// Whether the unroller may clone the loop body (`RAKIT_UNROLL_CLONE`).
pub fn unroll_clone() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_UNROLL_CLONE"))
}

/// Whether the unroller collapses trivial forwarding latches
/// (`RAKIT_NO_COLLAPSE` off).
pub fn collapse_forwarders_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_COLLAPSE"))
}

/// Rakit `opt_level` override (`RAKIT_OPT_LEVEL`).
pub fn opt_level() -> Option<&'static str> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| var("RAKIT_OPT_LEVEL")).as_deref()
}

/// Inliner growth budget as a percentage of the caller size (`RAKIT_INLINE_GROWTH`).
pub fn inline_growth_pct() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("RAKIT_INLINE_GROWTH") {
        Some(v) => v.parse().unwrap_or(crate::passes::inliner::GROWTH_PCT),
        None => crate::passes::inliner::GROWTH_PCT,
    })
}

/// Maximum number of CLIF instructions for an RIR round-trip when explicitly set.
/// `None` leaves the decision to the large-CGU budget below.
pub fn rir_max_insts() -> Option<usize> {
    static V: OnceLock<Option<usize>> = OnceLock::new();
    *V.get_or_init(|| std::env::var("RAKIT_RIR_MAX_INSTS").ok()?.parse().ok())
}

/// Whether `RAKIT_RIR_FORCE` is set: validation mode that exports even when no
/// RIR pass changed the function and ignores the per-CGU size budget.
pub fn rir_force() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_RIR_FORCE"))
}

/// A CGU with more than this many RIR-eligible functions is treated as large.
pub fn rir_large_cgu_functions() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("RAKIT_RIR_LARGE_CGU").ok().and_then(|v| v.parse().ok()).unwrap_or(64)
    })
}

/// In a large CGU, only round-trip functions at or below this size.  The RIR
/// pipeline cost is per function and does not pay off for large bodies whose
/// hot loops are already handled by the codegen passes.
pub fn rir_large_cgu_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("RAKIT_RIR_LARGE_MAX_INSTS").ok().and_then(|v| v.parse().ok()).unwrap_or(50)
    })
}
