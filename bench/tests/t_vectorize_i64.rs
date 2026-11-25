//! i64 lanes: add loop in an address-taken `fn i64ops` (per-function `paddq`).
//! per_vec is 2, so exercise even (n=1000), odd tail (n=999 -> 1), sub-vector
//! (n=1) and zero-trip (n=0); all must equal LLVM.
#[no_mangle]
pub unsafe extern "C" fn i64ops(pa: *const u64, pb: *const u64, pc: *mut u64, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_add(*pb.add(i));
        i += 1;
    }
}
fn ck(v: &[u64]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    for &raw in &[1000usize, 999, 3, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u64> = (0..n).map(|i| (i as u64).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u64> = (0..n).map(|i| (i as u64).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0u64; n];
        let f: unsafe extern "C" fn(*const u64, *const u64, *mut u64, usize) = i64ops;
        unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        println!("n={} add={}", n, ck(&c));
    }
}
