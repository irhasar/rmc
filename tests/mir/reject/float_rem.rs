#![no_std]
#![no_main]
include!("../prelude.rs");

// Float remainder is a *rejected* case. The note must not claim "floating-point
// types are not implemented" (they are; see `float.rs`), only that `%` is.
// rustc compiles the reference.
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(1) as f64;
    out((a % 2.0) as i64);
    0
}
