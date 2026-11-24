#![no_std]
#![no_main]
include!("prelude.rs");

// A bounds check may be dropped only on a fact that still holds. `d = x & 15`
// bounds `d` below 16, but the bound was kept after `d = get(v)` (a call
// writing its destination was never treated as a write) and after
// `*p = v` through `p = &mut d` (a write through a pointer names no local).
// Either way `t[d]` read past the array with `d = 1005` instead of panicking.

use core::hint::black_box;

#[inline(never)]
fn get(k: i64) -> i64 {
    black_box(k)
}

#[inline(never)]
fn after_call(x: i64, v: i64) -> i64 {
    let t: [i64; 16] = [7; 16];
    let mut d = x & 15;
    let s = t[d as usize];
    d = get(v);
    s + t[d as usize]
}

#[inline(never)]
fn through_pointer(x: i64, v: i64) -> i64 {
    let t: [i64; 16] = [7; 16];
    let mut d = x & 15;
    let p = &mut d;
    *p = black_box(v);
    t[d as usize]
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let x = arg(1);
    let v = arg(2);
    let which = arg(3);
    if which == 0 {
        out(after_call(x, v));
    } else {
        out(through_pointer(x, v));
    }
    0
}
