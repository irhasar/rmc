#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let p = alc(64);
    unsafe {
        *((p) as *mut i64) = 12345;
        out(*((p) as *mut i64));
        *((p) as *mut i64) = arg(2);
        out(*((p) as *mut i64));
    }
    rel(p);
    out(1);
    0
}
