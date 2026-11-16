#![no_std]
#![no_main]
include!("prelude.rs");

// `lo <= x && x <= hi` folds into one unsigned comparison of `x - lo`
// (`fold_range_checks`). Each shape below: both orders, strict and non-strict
// bounds, negated tests, negative bounds on signed types, narrow and wide
// unsigned types, a bound that is also an array guard, and a pair that is not
// an interval and must keep both tests. Every value arrives at run time and is
// swept across both ends of each range.

#[inline(never)]
fn classify(x: i64) -> i64 {
    let mut r = 0;
    let c = x as u8;
    if c >= b'0' && c <= b'9' {
        r += 1;
    }
    if x > -5 && x < 10 {
        r += 10;
    }
    if x <= 100 && x >= 50 {
        r += 100;
    }
    if !(x < 3) && !(x > 7) {
        r += 1000;
    }
    let s = x as i8;
    if s >= -100 && s <= -90 {
        r += 10000;
    }
    let u = x as u64;
    if u > 18446744073709551000 && u < 18446744073709551600 {
        r += 100000;
    }
    // Not an interval: both tests stay.
    if x < 20 || x > 30 {
        r += 1000000;
    }
    r
}

// The guard of an index: the bounds check stays provable or is kept.
#[inline(never)]
fn guarded(t: &[i64; 8], i: i64) -> i64 {
    if i >= 0 && i < 8 {
        t[i as usize]
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let mut acc = 0i64;
    let mut x = k - 120;
    while x < k + 120 {
        acc = acc.wrapping_mul(3).wrapping_add(classify(x));
        x += 1;
    }
    out(acc);
    let t = [3, 1, 4, 1, 5, 9, 2, 6];
    let mut g = 0;
    let mut i = -3;
    while i < 11 {
        g = g * 7 + guarded(&t, i + k % 3);
        i += 1;
    }
    out(g);
    0
}
