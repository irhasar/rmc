//! forloops benchmark twin (see common.rs).
#![no_std]
#![no_main]

include!("common.rs");

struct Acc {
    sum: i64,
    cnt: i64,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut a: Acc = Acc { sum: 0, cnt: 0 };
    // The `for` loops of the AST twin are written as `while` loops here:
    // `rustc --emit=mir` on a single crate lowers a range `for` through
    // `core`'s iterator machinery (`Range::next`, `Option`, `discriminant`),
    // which the MIR dump does not carry, while a `while` loop is pure control
    // flow. The iteration order and body are identical.
    let mut i: i64 = 0;
    while i < n {
        a.sum += i;
        if a.sum > 1000000000 {
            a.sum -= 1000000000;
            a.cnt += 1;
        }
        i += 1;
    }
    i = 0;
    while i <= 1000 {
        let mut j: i64 = 0;
        while j <= 10 {
            a.sum += i ^ j;
            j += 1;
        }
        i += 1;
    }
    print(a.sum);
    print(a.cnt);
    0
}
