//! `-Ctarget-feature=+avx2` / `+fma` must reach the Cranelift ISA builder *with*
//! the implied `+avx`: Cranelift gates every VEX encoding on `has_avx`, so
//! without the widening the flag is accepted and the backend silently keeps the
//! SSE2 lowering. regressions.sh checks the encoding, not just the output.
#![feature(portable_simd)]
use std::simd::Simd;

#[inline(never)]
fn sum(a: &[i32]) -> i32 {
    let mut acc = Simd::<i32, 4>::splat(0);
    for c in a.chunks_exact(4) {
        acc += Simd::from_slice(c);
    }
    acc.to_array().iter().sum()
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let v: Vec<i32> = (0..n).map(|i| i as i32).collect();
    println!("{}", sum(&v));
}
