//! heavy benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 1000000000;
    }
    let mut sum: i64 = 0;
    let mut alt: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        sum = sum + i;
        if sum > 1000000000 {
            sum -= 1000000000;
        }
        alt = alt + (i & 1);
        i += 1;
    }
    print(sum);
    print(alt);
    0
}
