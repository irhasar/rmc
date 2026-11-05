#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out((0xF0i64 | 0x0F) & 0xFF);
    out(0b1010i64 ^ 0b0110);
    out(!0i64);
    out(1i64 << 10);
    let x = 0xDEADBEEFi64;
    out(x >> 8);
    out(x << 4);
    out(arg(1) & 255);
    0
}
