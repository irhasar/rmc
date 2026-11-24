use std::time::Instant;
fn kernel(pa: *const u32, pb: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
        i += 1;
    }
}
fn main() {
    let n = 4_000_000usize;
    let a: Vec<u32> = (0..n as u32).map(|i| i.wrapping_mul(2654435761) ^ 0x1234_5678).collect();
    let b: Vec<u32> = (0..n as u32).map(|i| i.rotate_left(7) ^ 0x9e37_79b1).collect();
    let mut c = vec![0u32; n];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    for _ in 0..3 { kernel(pa, pb, pc, n); } // warmup
    let t = Instant::now();
    for _ in 0..20 { kernel(pa, pb, pc, n); }
    let ns = t.elapsed().as_nanos();
    let mut h = 0u32; for &x in &c { h = h.wrapping_mul(16777619) ^ x; }
    println!("kernel_ns={} sum={}", ns, h);
}
