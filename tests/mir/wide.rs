#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a: [i64; 100] = [7; 100];
    let mut s = 0i64;
    let mut i = 0usize;
    while i < 100 {
        s += a[i];
        i += 1;
    }
    out(s);
    out(a[0] + a[99]);
    0
}
