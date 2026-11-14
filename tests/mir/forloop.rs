#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut s = 0i64;
    for i in 0..10 {
        s += i;
    }
    out(s);
    let mut t = 0i64;
    for i in 0..arg(2) {
        t += i * 2;
    }
    out(t);
    0
}
