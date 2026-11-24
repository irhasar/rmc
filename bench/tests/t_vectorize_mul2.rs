//! Two-source u32 multiply: `c[i]=a[i]*b[i]` and `c[i]=(a[i]*b[i])*K`, in
//! address-taken fns (per-function pmuludq). Full/remainder/sub-vector/zero-trip.
#[no_mangle]
pub unsafe extern "C" fn mul2(pa: *const u32, pb: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_mul(*pb.add(i));
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn mul2k(pa: *const u32, pb: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_mul(*pb.add(i)).wrapping_mul(2654435761);
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
    for &raw in &[1000usize, 999, 5, 2, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0u32; n];
        let f: unsafe extern "C" fn(*const u32, *const u32, *mut u32, usize) = mul2;
        unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        let h1 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let f: unsafe extern "C" fn(*const u32, *const u32, *mut u32, usize) = mul2k;
        unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        let h2 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        println!("n={} mul2={} mul2k={}", n, h1, h2);
    }
}
