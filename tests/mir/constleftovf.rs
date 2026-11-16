#![no_std]
#![no_main]
include!("prelude.rs");

// The `.overflow` twin of `constleft`: with overflow checks on, rustc emits
// `SubWithOverflow(const k, x)` and the back end reads the flag with `seto`
// straight after the `sub`. Folding the `sub` into an `lea` both swapped the
// operands and left `seto` reading flags no instruction had set, so an
// overflowing `k - x` could pass and an in-range one could trap.
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let x = arg(1);
    out(5 - x);
    out(-1 - x);
    out(7 + x);
    0
}
