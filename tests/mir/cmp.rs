#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    outb(3 < 5);
    outb(5 < 3);
    outb(3 <= 3);
    outb(4 == 4);
    outb(4 != 4);
    outb(2 > 1);
    outb(1 >= 2);
    let a = arg(1);
    outb(a > 0);
    0
}
