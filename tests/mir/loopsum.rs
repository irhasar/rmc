#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(1);
    let mut s = 0i64;
    let mut i = 0i64;
    while i < n {
        s += i;
        i += 1;
    }
    out(s);
    0
}
