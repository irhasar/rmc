//! f32 lanes: add and mul loops in one address-taken `fn f32ops` (per-function
//! `addps`/`mulps` assertions). Exercises full vectors, remainders (n=999 -> 3),
//! sub-vector (n=2), and zero-trip (n=0); all results must equal LLVM (IEEE).
#[no_mangle]
pub unsafe extern "C" fn f32ops(
    pa: *const f32, pb: *const f32, pc: *mut f32,
    qa: *const f32, qb: *const f32, qc: *mut f32, n: usize,
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
fn ck(v: &[f32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x.to_bits() as u64).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    for &raw in &[1000usize, 999, 5, 2, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<f32> = (0..n).map(|i| (i as f32) * 0.5 + 1.25).collect();
        let b: Vec<f32> = (0..n).map(|i| (i as f32) * 0.25 - 0.5).collect();
        let mut c1 = vec![0f32; n];
        let mut c2 = vec![0f32; n];
        let f: unsafe extern "C" fn(*const f32, *const f32, *mut f32, *const f32, *const f32, *mut f32, usize) =
            f32ops;
        unsafe { f(a.as_ptr(), b.as_ptr(), c1.as_mut_ptr(), a.as_ptr(), b.as_ptr(), c2.as_mut_ptr(), n) };
        println!("n={} add={} mul={}", n, ck(&c1), ck(&c2));
    }
}
