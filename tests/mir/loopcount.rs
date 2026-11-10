#![no_std]
#![no_main]
include!("prelude.rs");

// Exercises `close_counted_loops`: a loop-invariant addend, a zero/negative
// bound, a negative addend, a pre-loop accumulator, and an addend read from
// memory. All four must match rustc, including the zero-trip guard.
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(0);
    let m = arg(1);
    let k = arg(2);

    let mut acc = 0i64;
    for _i in 0..n { acc += 7; }
    out(acc);

    let mut a2 = 100i64;
    for _i in 0..m { a2 += 3; }
    out(a2);

    let mut a3 = 0i64;
    for _i in 0..n { a3 += -5; }
    out(a3);

    let mut a4 = 0i64;
    for _i in 0..n { a4 += k; }
    out(a4);

    // Zero-trip and one-trip bounds.
    let mut a5 = 1i64;
    for _i in 0..0 { a5 += 9; }
    out(a5);
    let mut a6 = 1i64;
    for _i in 0..1 { a6 += 9; }
    out(a6);
    0
}
