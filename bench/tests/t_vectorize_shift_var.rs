//! Uniform runtime shift amounts: `c[i] = a[i] << s` / `>> s` where `s` is a
//! loop-invariant scalar (splatted). Covers the Cranelift `mask_xmm_shift`
//! `splat`-of-value rule (lane 0 as the masked count), u32 (pslld/psrld/psrad)
//! and u64 (psllq). Amounts include 0 and width-1 to check masking.
#[no_mangle]
pub unsafe extern "C" fn u32shlv(pa: *const u32, pc: *mut u32, n: usize, s: u32) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) << s;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn u32shrv(pa: *const u32, pc: *mut u32, n: usize, s: u32) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) >> s;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn i32sarv(pa: *const i32, pc: *mut i32, n: usize, s: u32) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) >> s;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn u64shlv(pa: *const u64, pc: *mut u64, n: usize, s: u64) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) << s;
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
type FU32 = unsafe extern "C" fn(*const u32, *mut u32, usize, u32);
type FI32 = unsafe extern "C" fn(*const i32, *mut i32, usize, u32);
type FU64 = unsafe extern "C" fn(*const u64, *mut u64, usize, u64);
fn main() {
    for &(raw, s) in &[(1000usize, 0u32), (999, 3), (17, 31), (15, 7), (1, 5), (0, 2), (11, 32), (11, 33), (11, 63), (11, 100)] {
        let n = std::hint::black_box(raw);
        let sv = std::hint::black_box(s);
        let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(2654435761).wrapping_add(0x9e37_79b9)).collect();
        let mut o = vec![0u32; n];
        let fs: [FU32; 2] = [u32shlv, u32shrv];
        let mut h1 = [0u64; 3];
        for (k, f) in fs.iter().enumerate() {
            unsafe { f(a.as_ptr(), o.as_mut_ptr(), n, sv) };
            h1[k] = ck(&o.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        let si: Vec<i32> = (0..n).map(|i| (i as i32).wrapping_mul(-1640531535).wrapping_add(0x1234_5678)).collect();
        let mut oi = vec![0i32; n];
        let g: FI32 = i32sarv;
        unsafe { g(si.as_ptr(), oi.as_mut_ptr(), n, sv) };
        h1[2] = ck(&oi.iter().map(|x| *x as i64 as u64).collect::<Vec<_>>());
        let b: Vec<u64> = (0..n).map(|i| (i as u64).wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(0xdead_beef)).collect();
        let mut ob = vec![0u64; n];
        let h64: FU64 = u64shlv;
        unsafe { h64(b.as_ptr(), ob.as_mut_ptr(), n, (sv as u64) & 63) };
        println!("n={} s={} {} {} {} {}", n, sv, h1[0], h1[1], h1[2], ck(&ob));
    }
}
