//! mix benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

#[inline(never)]
fn gcd(a: i64, b: i64) -> i64 {
    let mut x: i64 = a;
    let mut y: i64 = b;
    while y != 0 {
        let t: i64 = x % y;
        x = y;
        y = t;
    }
    x
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 20000;
    }
    let mut sum: i64 = 0;
    let mut i: i64 = 1;
    while i < n {
        sum = sum + gcd(i, 2 * i + 3);
        i = i + 1;
    }
    print(sum);
    0
}
