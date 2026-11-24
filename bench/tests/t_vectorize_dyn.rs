//! Dynamic trip bound: `n` is opaque at compile time (`black_box`), so the loop
//! can only vectorize through the runtime-bound path (not the constant path).
//! This file has exactly one xor+imul loop, so `pmuludq` in the binary proves it
//! vectorized; the runtime disjointness guard and `i+4 <= n` gate must still be
//! correct for a non-constant n.
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    let n = std::hint::black_box(1000usize);
    let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let b: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
    let mut c = vec![0u32; n];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    let mut i = 0usize;
    while i < n {
        unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
        i += 1;
    }
    println!("dyn {} n={}", ck(&c), n);
}
