// Kernel-only timing probe: times init, the 20 kernel passes, and the hash
// SEPARATELY (no subtraction between whole-process runs). Prints ns per phase.
// Correctness check: `sum` must match between L and M. Needs BOTH builds:
//   rustc +nightly-2026-11-17 -O bench/l3/ktime.rs -o ktime_L
//   env -u RAKIT_HOST_LLVM rakit/dist/bin/rustc-rakit --target <host> \
//       -C opt-level=3 bench/l3/ktime.rs -o ktime_M --edition 2021
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
    let t0 = Instant::now();
    let a: Vec<u32> = (0..n as u32).map(|i| i.wrapping_mul(2654435761) ^ 0x1234_5678).collect();
    let b: Vec<u32> = (0..n as u32).map(|i| i.rotate_left(7) ^ 0x9e37_79b1).collect();
    let mut c = vec![0u32; n];
    let init = t0.elapsed().as_nanos();
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    for _ in 0..3 { kernel(pa, pb, pc, n); } // warmup (not timed)
    let t1 = Instant::now();
    for _ in 0..20 { kernel(pa, pb, pc, n); }
    let kern = t1.elapsed().as_nanos();
    let t2 = Instant::now();
    let mut h = 0u32;
    for &x in &c { h = h.wrapping_mul(16777619) ^ x; }
    let hash = t2.elapsed().as_nanos();
    println!("init_ns={} kernel_ns={} hash_ns={} sum={}", init, kern, hash, h);
}
