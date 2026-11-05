#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut s = 0i64;
    let mut i = 0i64;
    while i < 10 {
        if i % 2 == 0 {
            s += i;
        } else {
            s -= 1;
        }
        i += 1;
    }
    out(s);
    let mut k = 0i64;
    loop {
        if k >= 5 {
            break;
        }
        k += 1;
    }
    out(k);
    0
}
