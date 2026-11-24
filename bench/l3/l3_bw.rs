// Reference: trivial vectorized tri-stream (identical memory traffic to L3).
// Position-dependent inputs + hash over ALL of `c`.
fn main() {
    let n = 4_000_000usize;
    let a: Vec<u32> = (0..n as u32).map(|i| i.wrapping_mul(2654435761)).collect();
    let b: Vec<u32> = (0..n as u32).map(|i| i.rotate_left(11)).collect();
    let mut c = vec![0u32; n];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    for _r in 0..20 {
        let mut i = 0usize;
        while i < n {
            unsafe { *pc.add(i) = (*pa.add(i)).wrapping_add(*pb.add(i)); }
            i += 1;
        }
    }
    let mut h = 0u32;
    for &x in &c { h = h.wrapping_mul(16777619) ^ x; }
    println!("{}", h);
}
