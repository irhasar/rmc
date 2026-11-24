//! Opt-in wall-clock accounting for rakit's own RAKIT passes.
//!
//! Enabled by `RAKIT_PASS_TIME=1`. Each pass accumulates nanoseconds in a
//! thread-local cell; [`report`] prints the per-thread totals. Used to keep the
//! compile-time cost of the custom passes honest without a profiler.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Read once: `std::env::var_os` takes a global lock, and `time()` is called for
/// nearly every function, so re-reading the variable per call shows up in a
/// normal (accounting-off) build.
static ENABLED: OnceLock<bool> = OnceLock::new();

/// Codegen runs on rustc's worker threads, so the totals have to be shared: a
/// thread-local accumulator would be reset before [`report`] ever ran.
static ACC: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];

pub const STORE_FORWARD: usize = 0;
pub const UNROLL: usize = 1;
pub const INLINE: usize = 2;
pub const COLLECT: usize = 3;
pub const IF_CONVERT: usize = 4;
pub const DIV_TEST: usize = 5;
pub const ROTATE: usize = 6;
pub const REC_LOOP: usize = 7;
pub const SINCOS: usize = 8;

const NAMES: [&str; 9] = [
    "store_forward",
    "unroll",
    "inline",
    "collect",
    "if_convert",
    "div_test",
    "rotate",
    "rec_loop",
    "sincos",
];

fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var_os("RAKIT_PASS_TIME").is_some())
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
