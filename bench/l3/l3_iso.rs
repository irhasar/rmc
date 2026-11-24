// L3 isolation: disjoint buffers, trip count divisible by 4 (vector fast path).
// Position-dependent inputs (a lane swap/broadcast changes the checksum) and a
// hash over ALL of `c` so every store is checked.
fn main() {
    let n = 4_000_000usize;
    let a: Vec<u32> = (0..n as u32).map(|i| i.wrapping_mul(2654435761) ^ 0x1234_5678).collect();
    let b: Vec<u32> = (0..n as u32).map(|i| i.rotate_left(7) ^ 0x9e37_79b1).collect();
    let mut c = vec![0u32; n];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    for _r in 0..20 {
        let mut i = 0usize;
        while i < n {
            unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
            i += 1;
        }
    }
    let mut h = 0u32;
    for &x in &c { h = h.wrapping_mul(16777619) ^ x; }
    println!("{}", h);
}
