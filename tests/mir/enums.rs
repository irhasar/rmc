#![no_std]
#![no_main]
include!("prelude.rs");

enum E {
    A,
    B(i64),
    C { v: i64 },
}

fn f(e: E) -> i64 {
    match e {
        E::A => 0,
        E::B(x) => x,
        E::C { v } => v * 2,
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(f(E::A));
    out(f(E::B(5)));
    out(f(E::C { v: 6 }));
    let tag = arg(3);
    let e = if tag > 0 { E::B(tag) } else { E::A };
    out(f(e));
    0
}
