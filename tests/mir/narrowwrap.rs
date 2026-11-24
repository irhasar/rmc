#![no_std]
#![no_main]
include!("prelude.rs");

// A narrow signed result that wraps must not keep a non-negative bound. With
// `x = 127`, `x + 1` in an `i8` is `-128`; the `usize` cast is huge, so the
// bounds check must stay and the program must exit 101 like rustc. The
// `masked` shape derives `x` from an opaque input, which is the form that
// gives `rv_range` a bound it could wrongly keep; the `literal` shape is the
// report's own program.

#[inline(never)]
fn literal(a: &[u8; 4], k: i64) -> u8 {
    let x: i8 = 127;
    let i = x.wrapping_add(core::hint::black_box(k as i8)) as usize;
    a[i]
}

#[inline(never)]
fn masked(a: &[u8; 129], k: i64) -> u8 {
    // `x` comes from an opaque input, so rustc cannot fold the add; the addend
    // is a constant, so `rv_range` infers the bound 128 that must be discarded
    // for an `i8` destination.
    let x: i8 = (k & 127) as i8;
    let i = x.wrapping_add(1) as usize;
    a[i]
}

#[inline(never)]
fn masked_two(a: &[u8; 129], k: i64) -> u8 {
    // The report's exact shape: mask to 126, add 2 in `i8`, cast to `usize`.
    // `126 + 2` wraps to -128, so the analysis must not carry a non-negative
    // bound through the cast and drop the check.
    let x: i8 = core::hint::black_box(k as i8);
    let i = (x & 126i8).wrapping_add(2) as usize;
    a[i]
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let a = [1u8; 4];
    let b = [2u8; 129];
    if k == 127 {
        out(masked(&b, k) as i64);
    } else {
        out(literal(&a, k) as i64);
    }
    out(masked_two(&b, k) as i64);
    0
}
