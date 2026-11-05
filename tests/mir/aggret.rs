#![no_std]
#![no_main]
include!("prelude.rs");

struct V {
    a: i64,
    b: i64,
}

fn mk(x: i64) -> V {
    V { a: x, b: x * 2 }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let v = mk(21);
    out(v.a + v.b);
    let w = mk(arg(5));
    out(w.a + w.b);
    0
}
