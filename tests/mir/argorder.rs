#![no_std]
#![no_main]
include!("prelude.rs");

// A call argument that lands in a register must survive the evaluation of the
// arguments after it: `f(7, arg(1))` and `f(arg(1), 7)` differ only in order.
#[inline(never)]
fn f(x: i64, y: i64) -> i64 { x * 10 + y }

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(1);
    out(f(7, a));
    out(f(a, 7));
    0
}
