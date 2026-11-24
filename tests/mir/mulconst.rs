#![no_std]
#![no_main]
include!("prelude.rs");

// Multiplication by a constant is a shift, a `lea`, or a `lea` and a shift
// when the constant allows (`x * 10` is `lea (x,x,4)` then `x + x`). Every
// shape below, with the constant on either side, wrapping past 2^63, and on
// narrower types whose result is truncated afterwards. The factor arrives at
// run time.

#[inline(never)]
fn wide(x: i64) -> i64 {
    let mut h: i64 = 0;
    h = h.wrapping_mul(31) ^ x.wrapping_mul(1);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(2);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(3);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(5);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(9);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(6);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(10);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(12);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(18);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(20);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(24);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(36);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(40);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(72);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(1 << 40);
    h = h.wrapping_mul(31) ^ 10i64.wrapping_mul(x);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(7);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(-10);
    h = h.wrapping_mul(31) ^ x.wrapping_mul(0);
    h
}

#[inline(never)]
fn unsigned(x: u64) -> i64 {
    let a = x.wrapping_mul(10);
    let b = x.wrapping_mul(1 << 63);
    let c = 9u64.wrapping_mul(x);
    (a ^ (b >> 7) ^ (c << 5) ^ (c >> 59)) as i64
}

#[inline(never)]
fn narrow(x: i64) -> i64 {
    let a = (x as u8).wrapping_mul(10);
    let b = (x as i32).wrapping_mul(12);
    let c = (x as i16).wrapping_mul(9);
    let d = (x as u32).wrapping_mul(40);
    a as i64 + b as i64 + c as i64 + d as i64
}

// The `textparse` shape: an accumulation through a multiply by ten.
#[inline(never)]
fn digits(x: i64) -> i64 {
    let mut cur: i64 = 0;
    let mut v = x;
    let mut i = 0;
    while i < 25 {
        cur = cur.wrapping_mul(10).wrapping_add(v & 7);
        v = (v << 3) ^ ((v as u64) >> 61) as i64;
        i += 1;
    }
    cur
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    out(wide(k));
    out(unsigned(k as u64));
    out(narrow(k));
    out(digits(k));
    0
}
