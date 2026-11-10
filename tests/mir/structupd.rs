#![no_std]
#![no_main]
include!("prelude.rs");

struct P {
    x: i64,
    y: i64,
    z: i64,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let b = P { x: 1, y: 2, z: 3 };
    let c = P { y: 9, ..b };
    out(c.x + c.y + c.z);
    let d = P { ..c };
    out(d.x + d.y + d.z);
    0
}
