#![no_std]
#![no_main]
include!("prelude.rs");
fn g(x: i64) -> i64 { x * 2 }
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(0);
    // IV-dependent addend: must not close.
    let mut a = 0i64;
    for i in 0..n { a += i; }
    out(a);
    // Two accumulators: not the single-acc shape.
    let mut b = 0i64;
    let mut c = 0i64;
    for _i in 0..n { b += 1; c += 2; }
    out(b); out(c);
    // Body with a call: side effect.
    let mut d = 0i64;
    for i in 0..n { d += g(i); }
    out(d);
    // IV read after loop: final value observable.
    let mut iv_final = 0i64;
    let mut e = 0i64;
    for i in 0..n { e += 1; iv_final = i; }
    out(e); out(iv_final);
    // Nested loop.
    let mut f = 0i64;
    for _i in 0..n { for _j in 0..3 { f += 1; } }
    out(f);
    0
}
