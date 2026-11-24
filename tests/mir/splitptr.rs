#![no_std]
#![no_main]
include!("prelude.rs");

// `split_loop_ranges` saves each value an inner loop never mentions, right
// after the entry block's last mention of it, and sinks the entry block's
// constant assignments (`i = 0`) below those saves. A constant assignment
// commutes with every statement that does not mention its destination, but a
// local whose address was taken is mentioned by a read through a pointer to it
// (`*p`) without its own name appearing. Sinking `x = 2` below `v = *p + param`
// made `v` read the old `x`.
//
// The loop calls `black_box` each trip, so the live values across the call
// exceed the callee-saved registers and the split is taken.

#[inline(never)]
fn through_pointer(a: i64, b: i64, c: i64, d: i64, param: i64, n: i64) -> i64 {
    let mut x = 1i64;
    let p = core::hint::black_box(&raw mut x);
    x = 2;
    let v = unsafe { *p } + param;
    let mut i = 0i64;
    while i < n {
        core::hint::black_box(i);
        i += 1;
    }
    v + a + b + c + d
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let n = arg(2);
    out(through_pointer(k, k + 1, k + 2, k + 3, k * 5, n));
    0
}
