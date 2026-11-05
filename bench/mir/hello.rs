//! hello benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

// `print_str` reads rmc's own string layout: an `i64` length followed by the
// bytes. A Rust `&[u8]` is a fat pointer with the opposite field order, so the
// twin spells the record out to pass the same address the AST twin does.
#[repr(C)]
struct Str {
    len: i64,
    bytes: [u8; 15],
}

static MSG: Str = Str { len: 14, bytes: *b"hello from rmc\0" };

#[no_mangle]
pub extern "C" fn main() -> i64 {
    unsafe { print_str(&MSG as *const Str as *const u8) };
    print(42);
    0
}
