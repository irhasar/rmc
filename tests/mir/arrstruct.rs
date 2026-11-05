#![no_std]
#![no_main]
include!("prelude.rs");

struct P {
    x: i64,
    y: i64,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let ps = [P { x: 1, y: 2 }, P { x: 3, y: 4 }];
    let mut s = 0i64;
    let mut i = 0usize;
    while i < 2 {
        s += ps[i].x + ps[i].y;
        i += 1;
    }
    out(s);
    0
}
