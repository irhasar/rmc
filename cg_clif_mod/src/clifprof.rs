//! Opt-in wall-clock accounting for Cranelift's own passes.
//!
//! cg_clif installs a [`Profiler`](cranelift_codegen::timing::Profiler) that
//! routes pass timings into rustc's self-profile. That is the right thing when
//! `-Zself-profile` is used, but it costs a `SelfProfilerRef` guard per pass even
//! when the user did not ask for a profile. With `CG_CLIF_CLIF_TIME=1` this
//! module installs a cheaper profiler instead: it accumulates nanoseconds per
//! pass in a thread-local array and prints one report per thread. Used to find
//! which Cranelift pass dominates compile time.

use std::any::Any;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use cranelift_codegen::timing::{ALL_PASSES, NUM_PASSES, Pass, Profiler};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the cheap per-pass profiler should be installed.
pub(crate) fn enabled() -> bool {
    if ENABLED.load(Ordering::Relaxed) {
        return true;
    }
    let on = std::env::var_os("CG_CLIF_CLIF_TIME").is_some();
    ENABLED.store(on, Ordering::Relaxed);
    on
}

thread_local! {
    static ACC: [Cell<u64>; NUM_PASSES] = [const { Cell::new(0) }; NUM_PASSES];
}

struct Guard {
    pass: Pass,
    start: Instant,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let ns = self.start.elapsed().as_nanos() as u64;
        ACC.with(|a| a[self.pass as usize].set(a[self.pass as usize].get() + ns));
    }
}

pub(crate) struct PassProfiler;

impl Profiler for PassProfiler {
    fn start_pass(&self, pass: Pass) -> Box<dyn Any> {
        Box::new(Guard { pass, start: Instant::now() })
    }
}

/// Print the calling thread's per-pass totals in descending order.
pub(crate) fn report() {
    if !enabled() {
        return;
    }
    ACC.with(|a| {
        let mut rows: Vec<(u64, Pass)> = ALL_PASSES
            .iter()
            .map(|&pass| (a[pass as usize].replace(0), pass))
            .filter(|&(ns, _)| ns > 0)
            .collect();
        rows.sort_unstable_by(|x, y| y.0.cmp(&x.0));
        let total: u64 = rows.iter().map(|r| r.0).sum();
        let mut s = format!("[cliftime] total={:.2}ms", total as f64 / 1e6);
        for (ns, pass) in rows {
            s.push_str(&format!(" {}={:.2}ms", pass.description(), ns as f64 / 1e6));
        }
        eprintln!("{s}");
    });
}
