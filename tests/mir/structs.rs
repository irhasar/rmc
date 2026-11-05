#![no_std]
#![no_main]
include!("prelude.rs");

struct P {
    x: i64,
    y: i64,
}

fn area(p: P) -> i64 {
    p.x * p.y
}

fn bump(p: &mut P) {
    p.x += 1;
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let p = P { x: 3, y: 4 };
    out(area(p));
    let mut q = P { x: 10, y: 20 };
    bump(&mut q);
    out(q.x + q.y);
    0
}
