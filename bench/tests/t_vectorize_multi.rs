//! Two independent eligible loops in one function. `do_vectorize` must vectorize
//! *both* (an early exit after the first match would leave the second scalar).
//! Each loop lowers to 2 `pmuludq` on SSE2, so a correct build has >= 4; the
//! regression harness asserts that count.
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    let n = std::hint::black_box(1000usize);
    let a1: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let b1: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
    let mut c1 = vec![0u32; n];
    let (pa, pb, pc) = (a1.as_ptr(), b1.as_ptr(), c1.as_mut_ptr());
    let mut i = 0usize;
    while i < n {
        unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
        i += 1;
    }
    let a2: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(17) ^ 0xabcd).collect();
    let b2: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(29) ^ 0x55).collect();
    let mut c2 = vec![0u32; n];
    let (qa, qb, qc) = (a2.as_ptr(), b2.as_ptr(), c2.as_mut_ptr());
    let mut j = 0usize;
    while j < n {
        unsafe { *qc.add(j) = (*qa.add(j) ^ *qb.add(j)).wrapping_mul(40503); }
        j += 1;
    }
    println!("m1 {} m2 {}", ck(&c1), ck(&c2));
}
