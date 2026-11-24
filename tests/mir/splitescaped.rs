#![no_std]
#![no_main]
include!("prelude.rs");

// `split_loop_ranges` saves values across a register-dense loop and sinks the
// entry block's constant assignments below those saves. A read or write through
// a pointer never names the local it touches, so the order of the pointer
// access and a sunk assignment matters even when the assignment's own local is
// the only one the scan sees. Here `x`'s address is taken, `x = 5` must stay
// above the `*p` read, and the inner loop keeps enough values live across its
// `black_box` call to force the split.

#[inline(never)]
fn through_pointer(a: i64, b: i64, c: i64, d: i64, param: i64, n: i64) -> i64 {
    let mut x = 0i64;
    let p = core::hint::black_box(&mut x as *mut i64);
    let mut i = 0i64;
    let mut v = 0i64;
    while i < n {
        x = 5;
        v = unsafe { *p } + param;
        let mut j = 0i64;
        while j < n {
            core::hint::black_box(j);
            j += 1;
        }
        i += 1;
    }
    v + a + b + c + d
}

#[inline(never)]
fn taken_after(a: i64, b: i64, c: i64, d: i64, param: i64, n: i64) -> i64 {
    let mut x = 0i64;
    x = 5;
    let p = core::hint::black_box(&mut x as *mut i64);
    let mut i = 0i64;
    let mut v = 0i64;
    while i < n {
        v = unsafe { *p } + param;
        let mut j = 0i64;
        while j < n {
            core::hint::black_box(j);
            j += 1;
        }
        i += 1;
    }
    v + a + b + c + d
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let n = arg(2);
    out(through_pointer(k, k + 1, k + 2, k + 3, k * 5, n));
    out(taken_after(k, k + 1, k + 2, k + 3, k * 5, n));
    0
}
