#![no_std]
#![no_main]
include!("prelude.rs");

// A type with its own `Drop` impl: the `drop(_N)` terminator must call it.
struct Guard { v: i64 }
impl Drop for Guard {
    fn drop(&mut self) { out(self.v); }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let g = Guard { v: arg(0) + 42 };
    out(1);
    0
}
