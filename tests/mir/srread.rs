#![no_std]
#![no_main]
include!("prelude.rs");

// Exercises the strength-reduce soundness guard. `strength_reduce` turns
// `p = i * C` into a running value advanced in the latch, but the advance also
// fires after the final increment, so the running value is one step ahead once
// the loop exits. When the multiply's destination is read after the loop it
// must not be reused (the first loop here); when it only feeds an accumulator
// read inside the loop, reuse is sound (the second loop).
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(0);

    // Destination read after the loop: must print (n - 1) * 7, not n * 7.
    let mut i = 0i64;
    let mut last = 0i64;
    while i < n {
        last = i * 7;
        i += 1;
    }
    out(last);

    // Destination feeds an accumulator read only inside the loop: reuse is
    // sound and the result is the same either way.
    let mut j = 0i64;
    let mut acc = 0i64;
    while j < n {
        acc += j * 7;
        j += 1;
    }
    out(acc);

    // Two distinct constants, one read after the loop and one not.
    let mut k = 0i64;
    let mut a = 0i64;
    let mut b = 0i64;
    while k < n {
        a = k * 3;
        b += k * 5;
        k += 1;
    }
    out(a);
    out(b);
    0
}
