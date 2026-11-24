//! Loop-invariant scalar broadcast: `c[i]=a[i]*k`, `c[i]=a[i]+k` (u32) and
//! `c[i]=a[i]*k` (f32) with a runtime scalar `k`, in address-taken fns
//! (per-function pmuludq/paddd/mulps). Full/remainder/sub-vector/zero-trip.
#[no_mangle]
pub unsafe extern "C" fn scalemul(pa: *const u32, pc: *mut u32, k: u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_mul(k);
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn scaleadd(pa: *const u32, pc: *mut u32, k: u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_add(k);
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn fscale(pf: *const f32, pout: *mut f32, k: f32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pout.add(i) = *pf.add(i) * k;
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
        let k = std::hint::black_box(2654435761u32);
        let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let mut c = vec![0u32; n];
        let f: unsafe extern "C" fn(*const u32, *mut u32, u32, usize) = scalemul;
        unsafe { f(a.as_ptr(), c.as_mut_ptr(), k, n) };
        let h1 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let f: unsafe extern "C" fn(*const u32, *mut u32, u32, usize) = scaleadd;
        unsafe { f(a.as_ptr(), c.as_mut_ptr(), k, n) };
        let h2 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let af: Vec<f32> = (0..n).map(|i| (i as f32) * 0.5 + 1.25).collect();
        let mut cf = vec![0f32; n];
        let kf = std::hint::black_box(1.5f32);
        let f: unsafe extern "C" fn(*const f32, *mut f32, f32, usize) = fscale;
        unsafe { f(af.as_ptr(), cf.as_mut_ptr(), kf, n) };
        let h3 = ck(&cf.iter().map(|x| x.to_bits() as u64).collect::<Vec<_>>());
        println!("n={} mul={} add={} fmul={}", n, h1, h2, h3);
    }
}
