#![no_std]
#![no_main]
include!("prelude.rs");

// The wrapping arithmetic family. rustc emits `wrapping_*` as a bare `Add` and
// its own range/iterator desugaring as `AddUnchecked`; the back end must treat
// both as the plain instruction and never add a trap of its own, or a `for`
// over a range would fault and `wrapping_add` would trap.
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let big: i64 = i64::MAX;
    out(big.wrapping_add(1));
    out(i64::MIN.wrapping_sub(1));
    out(i64::MIN.wrapping_mul(-1));
    out(i64::MAX.wrapping_mul(2));

    let z: u64 = 0;
    out(z.wrapping_sub(1) as i64);

    // A `for` over a range desugars to `AddUnchecked` on the counter.
    let mut s = 0i64;
    for i in 0..5 {
        s = s.wrapping_add(i * 3);
    }
    out(s);

    // The same counter feeding a wrapping multiply.
    let mut p = 1i64;
    for i in 1..5 {
        p = p.wrapping_mul(i);
    }
    out(p);

    // Shifts, including the wrapping spellings.
    out(1i64.wrapping_shl(62));
    out((-8i64).wrapping_shr(1));
    0
}
