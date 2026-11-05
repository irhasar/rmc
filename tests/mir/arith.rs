#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(2 + 3 * 4);
    out((2 + 3) * 4);
    out(20 - 5 - 3);
    out(100 / 7);
    out(100 % 7);
    out(-7 / 2);
    out(-7 % 2);
    out(7 / -2);
    out(0 - 5);
    let mut x: i64 = 17;
    x = x + 3;
    x = x * 2;
    x = x - 5;
    x = x / 3;
    out(x);
    0
}
