#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut i = 1i64;
    while i <= 5 {
        out(1000 / i);
        out(1000 % i);
        i += 1;
    }
    let n = arg(1);
    if n != 0 {
        out(999 / n);
    }
    0
}
