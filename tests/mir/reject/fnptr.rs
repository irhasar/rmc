#![no_std]
#![no_main]
include!("../prelude.rs");

// A function pointer is a *rejected* case: taking a function as a value and
// calling it through the pointer needs an indirect call, which is not
// implemented. rustc compiles the reference, so the suite asserts a working
// reference and a clean, named error from rmc.
fn twice(x: i64) -> i64 {
    x * 2
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let f: fn(i64) -> i64 = twice;
    out(f(arg(1)));
    0
}
