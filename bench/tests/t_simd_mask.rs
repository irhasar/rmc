// SIMD comparisons may return a mask whose lane *width* differs from the input
// (`i32x8` compared into `i8x8` or `i64x8`). A single vector compare fills the
// input-width lanes, so the split/vector fast paths must fall back when the
// mask shape differs.
#![feature(core_intrinsics, portable_simd)]
use core::intrinsics::simd::{simd_eq, simd_lt};
use std::simd::Simd;

fn main() {
    let a = Simd::<i32, 8>::from_array([1, 2, 3, 4, -1, -2, 0, 7]);
    let b = Simd::<i32, 8>::from_array([1, 0, 3, 5, -1, 2, 0, -7]);

    let narrow: Simd<i8, 8> = unsafe { simd_eq(a, b) };
    println!("i8x8 eq {:?}", narrow.to_array());

    let wide: Simd<i64, 8> = unsafe { simd_lt(a, b) };
    println!("i64x8 lt {:?}", wide.to_array());

    // Same shape (the portable-simd API): must stay exact.
    let same: Simd<i32, 8> = unsafe { simd_lt(a, b) };
    println!("i32x8 lt {:?}", same.to_array());

    let wide64: Simd<u8, 2> = unsafe { simd_eq(Simd::<u64, 2>::splat(5), Simd::<u64, 2>::from_array([5, 6])) };
    println!("u8x2 eq {:?}", wide64.to_array());
}
