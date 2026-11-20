//! Process-wide tunables, read from the environment once.
//!
//! The CLIF passes run once per function, so a `std::env` lookup inside a pass
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

/// Whether the store-forwarding pass runs at all (`CG_CLIF_NO_STORE_FWD` off).
pub(crate) fn store_forward_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_NO_STORE_FWD"))
}

/// Whether the divisibility-test pass runs at all (`CG_CLIF_NO_DIVTEST` off).
pub(crate) fn div_test_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_NO_DIVTEST"))
}

/// Whether the if-conversion pass runs at all (`CG_CLIF_NO_IFCONV` off).
pub(crate) fn if_convert_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_NO_IFCONV"))
}

/// Per-function store-forwarding timing output (`CG_CLIF_SF_TIME`).
pub(crate) fn store_forward_time() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("CG_CLIF_SF_TIME"))
}

/// Substring of function names the store-forwarding pass skips (`CG_CLIF_SF_SKIP`).
pub(crate) fn store_forward_skip() -> Option<&'static str> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| var("CG_CLIF_SF_SKIP")).as_deref()
}

/// Whether the cross-block dataflow of store forwarding is enabled.
pub(crate) fn store_forward_cross_block() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_SF_NO_CBE"))
}

/// Whether `Rvalue::Repeat` may lower to a memset (`CG_CLIF_NO_REPEAT_MEMSET`).
pub(crate) fn repeat_memset() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_NO_REPEAT_MEMSET"))
}

/// Instruction-count budget above which the cross-block dataflow is skipped.
pub(crate) fn store_forward_cbe_max() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_SF_CBE_MAX") {
        Some(v) => v.parse().unwrap_or(usize::MAX),
        None => super::store_forward::CBE_MAX_INSTS,
    })
}

/// Largest callee (in CLIF instructions) the inliner accepts (`CG_CLIF_INLINE_MAX`).
pub(crate) fn inline_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_INLINE_MAX") {
        Some(v) => v.parse().unwrap_or(super::inliner::MAX_CALLEE_INSTS),
        None => super::inliner::MAX_CALLEE_INSTS,
    })
}

/// Largest *leaf* callee (no calls of its own) the inliner accepts
/// (`CG_CLIF_INLINE_LEAF_MAX`).
///
/// A leaf body is pure computation: once inlined, it exposes its arithmetic to
/// the caller's passes and lets the register allocator keep values in registers
/// across the former call boundary, which is where the runtime win comes from.
/// Non-leaf bodies mainly move a call around and can still be duplicated
/// cheaply, so they keep the smaller [`inline_max_insts`] cap.
pub(crate) fn inline_leaf_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_INLINE_LEAF_MAX") {
        Some(v) => v.parse().unwrap_or(super::inliner::MAX_LEAF_INSTS),
        None => super::inliner::MAX_LEAF_INSTS,
    })
}

/// Maximum times one callee may be inlined per caller (`CG_CLIF_INLINE_USES`).
pub(crate) fn inline_max_uses() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_INLINE_USES") {
        Some(v) => v.parse().unwrap_or(super::inliner::MAX_CALLEE_USES),
        None => super::inliner::MAX_CALLEE_USES,
    })
}

/// Whether trivial leaves bypass the per-caller use cap (`CG_CLIF_INLINE_NO_TRIVIAL` off).
pub(crate) fn inline_trivial_leaf() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_INLINE_NO_TRIVIAL"))
}

/// Instruction cap for the trivial-leaf exemption (`CG_CLIF_INLINE_TRIVIAL_MAX`).
pub(crate) fn inline_trivial_max_insts() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_INLINE_TRIVIAL_MAX") {
        Some(v) => v.parse().unwrap_or(super::inliner::TRIVIAL_LEAF_INSTS),
        None => super::inliner::TRIVIAL_LEAF_INSTS,
    })
}

/// Loop unroll factor (`CG_CLIF_UNROLL`).
pub(crate) fn unroll_factor() -> u64 {
    static V: OnceLock<u64> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_UNROLL") {
        Some(v) => v.parse().unwrap_or(super::unroll::UNROLL_FACTOR),
        None => super::unroll::UNROLL_FACTOR,
    })
}

/// Whether the unroller may clone the loop body (`CG_CLIF_UNROLL_CLONE`).
pub(crate) fn unroll_clone() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("CG_CLIF_UNROLL_CLONE"))
}

/// Whether the unroller collapses trivial forwarding latches
/// (`CG_CLIF_NO_COLLAPSE` off).
pub(crate) fn collapse_forwarders_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("CG_CLIF_NO_COLLAPSE"))
}

/// Cranelift `opt_level` override (`CG_CLIF_OPT_LEVEL`).
pub(crate) fn opt_level() -> Option<&'static str> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| var("CG_CLIF_OPT_LEVEL")).as_deref()
}

/// Inliner growth budget as a percentage of the caller size (`CG_CLIF_INLINE_GROWTH`).
pub(crate) fn inline_growth_pct() -> usize {
    static V: OnceLock<usize> = OnceLock::new();
    *V.get_or_init(|| match var("CG_CLIF_INLINE_GROWTH") {
        Some(v) => v.parse().unwrap_or(super::inliner::GROWTH_PCT),
        None => super::inliner::GROWTH_PCT,
    })
}
