#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a: [i64; 5] = [1, 2, 3, 4, 5];
    let mut s = 0i64;
    let mut i = 0usize;
    while i < 5 {
        s += a[i];
        i += 1;
    }
    out(s);
    let mut b = [0i64; 4];
    b[2] = 42;
    out(b[2] + b[0]);
    0
}
