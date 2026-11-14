//! stats benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

struct Acc {
    sum: i64,
    count: i64,
    max: i64,
    min: i64,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut a: Acc = Acc {
        sum: 0,
        count: 0,
        max: -9223372036854775807,
        min: 9223372036854775807,
    };
    let mut i: i64 = 1;
    while i < n {
        let v: i64 = (i * 2654435761) % 1000003;
        a.sum += v;
        a.count += 1;
        if v > a.max {
            a.max = v;
        }
        if v < a.min {
            a.min = v;
        }
        i += 1;
    }
    print(a.sum);
    print(a.max);
    print(a.min);
    print(a.count);
    0
}
