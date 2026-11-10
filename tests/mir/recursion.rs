#![no_std]
#![no_main]
include!("prelude.rs");

fn fib(n: i64) -> i64 {
    if n < 2 {
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

fn fact(n: i64) -> i64 {
    if n <= 1 {
        1
    } else {
        n * fact(n - 1)
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(fib(20));
    out(fact(10));
    out(fib(arg(4)));
    0
}
