#![no_std]
#![no_main]
include!("prelude.rs");

// More live locals than callee-saved registers, all read across a call in a
// loop: every one must keep its value, so the frame slots and the promotion
// pass must agree.
#[inline(never)]
fn f(x: i64, y: i64) -> i64 { x * 1000000 + y }

#[inline(never)]
fn g(a: i64) -> i64 {
    let mut s = 0;
    let mut i = 0;
    let mut v0 = a; let mut v1 = a + 1; let mut v2 = a + 2;
    let mut v3 = a + 3; let mut v4 = a + 4; let mut v5 = a + 5;
    let mut v6 = a + 6; let mut v7 = a + 7;
    while i < 2 {
        s = s + f(v7, v6) + f(v6, v7);
        s = s + v0 + v1 + v2 + v3 + v4 + v5 + v6 + v7;
        i = i + 1;
    }
    s
}

#[no_mangle]
pub extern "C" fn main() -> i64 { out(g(arg(1))); 0 }
