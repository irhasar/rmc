// Held-out SIMD: runtime-sized packed-integer multiply/add/shift loop.
use std::arch::x86_64::*;

fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let seed: u16 = std::env::args().nth(2).unwrap().parse().unwrap();
    let mut sum = 0u64;
    unsafe {
        let mut acc = _mm_set1_epi16(seed as i16);
        for i in 0..n {
            let x = _mm_set1_epi16((i as u16).wrapping_mul(seed) as i16);
            let y = _mm_mullo_epi16(acc, x);
            acc = _mm_add_epi16(acc, y);
            let z = _mm_srli_epi16(acc, 3);
            let mut lanes = [0u16; 8];
            _mm_storeu_si128(lanes.as_mut_ptr() as *mut __m128i, z);
            sum = sum.wrapping_add(lanes[(i & 7) as usize] as u64);
        }
    }
    println!("{n} {sum}");
}
