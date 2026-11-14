#![no_std]
#![no_main]
include!("prelude.rs");

// The `.overflow` marker compiles this with overflow checks on both sides.
// rustc then rewrites checked `+`/`-`/`*` into `AddWithOverflow` plus an
// `assert`, and the back end must honour that assert (exit 101) without adding
// a trap of its own. `arg` reads a runtime value so the overflow cannot be
// folded away at compile time.
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(0);
    let b = arg(1);
    out(a + b);
    out(a - b);
    out(a * b);
    0
}
