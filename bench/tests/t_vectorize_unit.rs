//! Single vectorizable loop (n=1000, unit stride, disjoint buffers). This file
//! contains exactly one `xor`+`imul` loop, so the presence of a packed multiply
//! (`pmuludq`) in the binary proves *this* loop was vectorized — it cannot be
//! satisfied by a tail loop elsewhere.
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    const N: usize = 1000;
    let a: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let b: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
    let mut c = vec![0u32; N];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    let mut i = 0usize;
    while i < N {
        unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
        i += 1;
    }
    println!("unit {}", ck(&c));
}
