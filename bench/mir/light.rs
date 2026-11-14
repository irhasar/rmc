//! light benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

const LEN: usize = 256;

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut table: [i64; LEN] = [0; LEN];
    let mut i: i64 = 0;
    while i < LEN as i64 {
        table[i as usize] = (i.wrapping_mul(2654435761)) % 1000003;
        i += 1;
    }
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let v = table[(i & 255) as usize];
        acc = acc + v;
        if acc > 1000000 {
            acc -= 1000000;
        }
        i += 1;
    }
    print(acc);
    0
}
