//! Opt-in wall-clock accounting for cg_clif's own CLIF passes.
//!
//! Enabled by `CG_CLIF_PASS_TIME=1`. Each pass accumulates nanoseconds in a
//! thread-local cell; [`report`] prints the per-thread totals. Used to keep the
//! compile-time cost of the custom passes honest without a profiler.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ACC: [Cell<u128>; 4] = [const { Cell::new(0) }; 4];
}

pub(crate) const STORE_FORWARD: usize = 0;
pub(crate) const UNROLL: usize = 1;
pub(crate) const INLINE: usize = 2;
pub(crate) const COLLECT: usize = 3;

const NAMES: [&str; 4] = ["store_forward", "unroll", "inline", "collect"];

fn enabled() -> bool {
    if ENABLED.load(Ordering::Relaxed) {
        return true;
    }
    let on = std::env::var_os("CG_CLIF_PASS_TIME").is_some();
    ENABLED.store(on, Ordering::Relaxed);
    on
}

/// Run `f`, charging its wall time to `slot` when accounting is on.
pub(crate) fn time<R>(slot: usize, f: impl FnOnce() -> R) -> R {
    if !enabled() {
        return f();
    }
    let t = std::time::Instant::now();
    let r = f();
    ACC.with(|a| a[slot].set(a[slot].get() + t.elapsed().as_nanos()));
    r
}

/// Print and reset the calling thread's totals.
pub(crate) fn report(where_: &str) {
    if !enabled() {
        return;
    }
    ACC.with(|a| {
        let mut s = format!("[passtime {where_}]");
        for (i, name) in NAMES.iter().enumerate() {
            let ns = a[i].replace(0);
            s.push_str(&format!(" {name}={:.1}ms", ns as f64 / 1e6));
        }
        eprintln!("{s}");
    });
}
