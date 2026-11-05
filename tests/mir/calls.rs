#![no_std]
#![no_main]
include!("prelude.rs");

fn add3(a: i64, b: i64, c: i64) -> i64 {
    a + b + c
}

fn muladd(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
    a * b + c * d + e * f
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(add3(1, 2, 3));
    out(add3(arg(1), arg(2), arg(3)));
    out(muladd(1, 2, 3, 4, 5, 6));
    0
}
