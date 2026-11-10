#![no_std]
#![no_main]
include!("prelude.rs");

struct Inner {
    a: i64,
    b: i64,
}

struct Outer {
    head: Inner,
    tail: i64,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let o = Outer {
        head: Inner { a: 1, b: 2 },
        tail: 3,
    };
    out(o.head.a + o.head.b + o.tail);
    0
}
