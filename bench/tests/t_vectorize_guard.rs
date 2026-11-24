//! Single loop whose vector guard is generated but never taken at runtime:
//! n=3 (< 4). The SIMD loop exists in the code but must execute zero vector
//! iterations. Buffers are padded to four elements and c[3] is seeded with a
//! sentinel, so a buggy guard that performs a four-lane store for n=3 would
//! overwrite c[3] and be detected (output-equality over three elements alone
//! could not see it). The presence of `pmuludq` proves the guard/vector body was
//! generated (not rejected).
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    const N: usize = 3;
    const PAD: usize = 4; // room for a (buggy) four-lane store to land
    let a: Vec<u32> = (0..PAD).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let b: Vec<u32> = (0..PAD).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
    let mut c = vec![0u32; PAD];
    c[3] = 0xDEAD_BEEF; // must stay unchanged: the guard must not store lane 3
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    let mut i = 0usize;
    while i < N {
        unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
        i += 1;
    }
    println!("guard {} tail4={}", ck(&c[..N]), c[3] == 0xDEAD_BEEF);
}
