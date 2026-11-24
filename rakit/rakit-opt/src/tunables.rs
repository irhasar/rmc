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

/// Per-function IR dump in the unroller (`RAKIT_UNROLL_DEBUG`).
pub fn unroll_debug() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_UNROLL_DEBUG"))
}

/// Lane-transform debug output (`RAKIT_LANE_DEBUG`).
pub fn lane_debug() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_LANE_DEBUG"))
}

/// Disable the u64->f64 narrowing rewrite (`RAKIT_NO_NARROW_FCVT`).
pub fn no_narrow_fcvt() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_NO_NARROW_FCVT"))
}

/// Whether the LCG lane transform runs (`RAKIT_NO_LANES` off).
pub fn lanes_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_LANES"))
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

/// Whether rotation applies to every matching loop (`RAKIT_ROTATE_ALL`), not
/// just call-bound ones.
pub fn rotate_all() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| flag("RAKIT_ROTATE_ALL"))
}

/// Whether the loop-rotation pass runs (`RAKIT_NO_ROTATE` disables it).
pub fn rotate_enabled() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !flag("RAKIT_NO_ROTATE"))
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
