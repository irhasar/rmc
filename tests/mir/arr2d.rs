#![no_std]
#![no_main]
include!("prelude.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut m: [[i64; 3]; 2] = [[1, 2, 3], [4, 5, 6]];
    m[1][2] = 9;
    let mut s = 0i64;
    let mut r = 0usize;
    while r < 2 {
        let mut c = 0usize;
        while c < 3 {
            s += m[r][c];
            c += 1;
        }
        r += 1;
    }
    out(s);
    0
}
