#![no_std]
#![no_main]
include!("prelude.rs");

// A comparison folded into a branch must use the operands' kind. The fused
// `cmp`+`jcc` used signed jumps for every type, so a `u64` above 2^63 compared
// as negative; and it compared floats as integer bit patterns, which is wrong
// for every negative value and for NaN (`NaN < x` was true). The materialised
// `bool` (`setb`/`ucomisd`) was already right, so only a comparison feeding a
// branch, a loop test or an assertion showed it. Every input arrives at run
// time through `arg`.

#[inline(never)]
fn unsigned_branches(k: i64) -> i64 {
    let x = (k as u64).wrapping_sub(7); // above 2^63 for k < 7
    let y = k as u64 + 2;
    let mut c = 0;
    if x > y {
        c += 1;
    }
    if y < x {
        c += 10;
    }
    if x >= y {
        c += 100;
    }
    if y <= x {
        c += 1000;
    }
    // A loop whose test is an unsigned comparison against a huge bound.
    let mut n: u64 = 0;
    let mut i: u64 = x;
    while i > y && n < 5 {
        n += 1;
        i = i.wrapping_add(1 << 62);
    }
    c * 10 + n as i64
}

#[inline(never)]
fn float_branches(k: i64) -> i64 {
    let a = k as f64 * -0.5;
    let b = k as f64 / 12.0;
    let nan = a / 0.0 * 0.0;
    let mut c = 0;
    if a < b {
        c += 1;
    }
    if b < a {
        c += 10;
    }
    if nan < b {
        c += 100;
    }
    if !(nan >= b) {
        c += 1000;
    }
    if nan == nan {
        c += 10000;
    }
    // A float loop test, counting down through negative values.
    let mut t = b;
    let mut n = 0;
    while t > a && n < 50 {
        t -= 0.25;
        n += 1;
    }
    c * 100 + n
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    out(unsigned_branches(k));
    out(float_branches(k));
    0
}
