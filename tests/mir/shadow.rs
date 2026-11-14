#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let x = 1i64;
    let x = x + 1;
    let x = x * 10;
    out(x);
    let mut acc = 0i64;
    let mut i = 0i64;
    while i < 4 {
        let d = i * i;
        acc += d;
        i += 1;
    }
    out(acc);
    0
}
