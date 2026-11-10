#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let x = 300u32;
    let y = x as u8;
    out(y as i64);
    let a = -1i32;
    out(a as i64);
    let b = 7i64;
    out(b as u32 as i64);
    out(true as i64);
    outb(0i64 != 0);
    0
}
