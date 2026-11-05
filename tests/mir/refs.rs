#![no_std]
#![no_main]
include!("prelude.rs");

fn bump(r: &mut i64) {
    *r += 1;
}

fn read(r: &i64) -> i64 {
    *r
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut x = 41i64;
    bump(&mut x);
    out(x);
    out(read(&x));
    let mut arr = [1i64, 2, 3];
    bump(&mut arr[1]);
    out(arr[1]);
    0
}
