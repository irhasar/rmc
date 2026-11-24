#![no_std]
#![no_main]
include!("../prelude.rs");

// expect: black_box

// `black_box::<&str>` moves two words in rustc (pointer and length); the
// runtime's identity call moves one, so rmc must refuse it rather than drop the
// length.

#[inline(never)]
fn len(s: &str) -> usize {
    core::hint::black_box(s).len()
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(len("abc") as i64 + arg(1));
    0
}
