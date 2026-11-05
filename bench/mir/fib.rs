//! fib benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

#[inline(never)]
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 30;
    }
    print(fib(n));
    0
}
