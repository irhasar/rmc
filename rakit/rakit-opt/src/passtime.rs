//! Opt-in wall-clock accounting for rakit's own RAKIT passes.
//!
//! Enabled by `RAKIT_PASS_TIME=1`. Each pass accumulates nanoseconds in a
//! thread-local cell; [`report`] prints the per-thread totals. Used to keep the
//! compile-time cost of the custom passes honest without a profiler.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Codegen runs on rustc's worker threads, so the totals have to be shared: a
/// thread-local accumulator would be reset before [`report`] ever ran.
static ACC: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

pub const STORE_FORWARD: usize = 0;
pub const UNROLL: usize = 1;
pub const INLINE: usize = 2;
pub const COLLECT: usize = 3;
pub const IF_CONVERT: usize = 4;
pub const DIV_TEST: usize = 5;

const NAMES: [&str; 6] = [
    "store_forward",
    "unroll",
    "inline",
    "collect",
    "if_convert",
    "div_test",
];

fn enabled() -> bool {
    if ENABLED.load(Ordering::Relaxed) {
        return true;
    }
    let on = std::env::var_os("RAKIT_PASS_TIME").is_some();
    ENABLED.store(on, Ordering::Relaxed);
    on
}

/// Run `f`, charging its wall time to `slot` when accounting is on.
pub fn time<R>(slot: usize, f: impl FnOnce() -> R) -> R {
    if !enabled() {
        return f();
    }
    let t = std::time::Instant::now();
    let r = f();
    ACC[slot].fetch_add(t.elapsed().as_nanos() as u64, Ordering::Relaxed);
    r
}


/// Print and reset the calling thread's totals.
pub fn report(where_: &str) {
    if !enabled() {
        return;
    }
    let mut s = format!("[passtime {where_}]");
    for (i, name) in NAMES.iter().enumerate() {
        let ns = ACC[i].swap(0, Ordering::Relaxed);
        s.push_str(&format!(" {name}={:.1}ms", ns as f64 / 1e6));
    }
    eprintln!("{s}");
}
