#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(1) > 0;
    let b = arg(2) > 0;
    outb(a && b);
    outb(a || b);
    outb(!a);
    outb(a & b);
    outb(a | b);
    outb(a ^ b);
    0
}
