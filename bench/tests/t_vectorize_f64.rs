//! f64 lanes: add and mul in one address-taken `fn f64ops` (per-function
//! `addpd`/`mulpd`). per_vec is 2; exercise full (1000), remainder (999 -> 1),
//! sub-vector (n=1) and zero-trip (n=0). All results must equal LLVM (IEEE).
#[no_mangle]
pub unsafe extern "C" fn f64ops(
    pa: *const f64, pb: *const f64, pc: *mut f64,
    qa: *const f64, qb: *const f64, qc: *mut f64, n: usize,
) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = *pa.add(i) + *pb.add(i);
        i += 1;
    }
    let mut j = 0usize;
    while j < n {
        *qc.add(j) = *qa.add(j) * *qb.add(j);
        j += 1;
    }
}
fn ck(v: &[f64]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x.to_bits()).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    for &raw in &[1000usize, 999, 5, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<f64> = (0..n).map(|i| (i as f64) * 0.5 + 1.25).collect();
        let b: Vec<f64> = (0..n).map(|i| (i as f64) * 0.25 - 0.5).collect();
        let mut c1 = vec![0f64; n];
        let mut c2 = vec![0f64; n];
        let f: unsafe extern "C" fn(*const f64, *const f64, *mut f64, *const f64, *const f64, *mut f64, usize) =
            f64ops;
        unsafe { f(a.as_ptr(), b.as_ptr(), c1.as_mut_ptr(), a.as_ptr(), b.as_ptr(), c2.as_mut_ptr(), n) };
        println!("n={} add={} mul={}", n, ck(&c1), ck(&c2));
    }
}
