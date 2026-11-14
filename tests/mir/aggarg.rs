#![no_std]
#![no_main]
include!("prelude.rs");

// A two-field struct passed by value: as a freshly built literal, as a bound
// local, and read after the call. A one-word copy would drop `y`.
#[derive(Clone, Copy)]
struct P2 { x: i64, y: i64 }

#[inline(never)]
fn sum2(p: P2) -> i64 { p.x * 1000 + p.y }

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(1);
    out(sum2(P2 { x: n, y: n + 2 }));
    let a = P2 { x: n, y: n + 2 };
    out(sum2(a));
    out(a.x);
    0
}
