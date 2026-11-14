//! sieve benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

const N: usize = 200000;

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut limit: i64 = arg(1);
    if limit == 0 {
        limit = 200000;
    }
    let mut flags: [i64; N] = [1; N];
    let mut count: i64 = 0;
    let mut i: i64 = 2;
    while i < limit {
        if flags[i as usize] == 1 {
            count = count + 1;
            let mut j: i64 = i * i;
            while j < limit {
                flags[j as usize] = 0;
                j = j + i;
            }
        }
        i = i + 1;
    }
    print(count);
    0
}
