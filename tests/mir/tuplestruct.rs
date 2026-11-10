#![no_std]
#![no_main]
include!("prelude.rs");

struct T(i64, i64);

fn sum(t: &T) -> i64 {
    t.0 + t.1
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let t = T(3, 4);
    out(sum(&t));
    out(t.0 * t.1);
    0
}
