//! Constant-shift vectorization: `c[i] = a[i] << K` / `>> K` for u32 (pslld/
//! psrld/psrad) and u64 (psllq/psrlq). Also covers the Cranelift fix: a splat
//! constant shift amount must lower via the immediate path, not `lo_gpr`.
#[no_mangle]
pub unsafe extern "C" fn u32shl(pa: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) << 3;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn u32shr(pa: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) >> 5;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn i32sar(pa: *const i32, pc: *mut i32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) >> 2;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn u64shl(pa: *const u64, pc: *mut u64, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) << 7;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn u64shr(pa: *const u64, pc: *mut u64, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) >> 9;
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
type F32 = unsafe extern "C" fn(*const u32, *mut u32, usize);
type FI32 = unsafe extern "C" fn(*const i32, *mut i32, usize);
type F64 = unsafe extern "C" fn(*const u64, *mut u64, usize);
fn main() {
    for &raw in &[1000usize, 999, 17, 15, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(2654435761).wrapping_add(0x9e37_79b9)).collect();
        let mut o = vec![0u32; n];
        let fs: [F32; 2] = [u32shl, u32shr];
        let mut h1 = [0u64; 3];
        for (k, f) in fs.iter().enumerate() {
            unsafe { f(a.as_ptr(), o.as_mut_ptr(), n) };
            h1[k] = ck(&o.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        let s: Vec<i32> = (0..n).map(|i| (i as i32).wrapping_mul(-1640531535).wrapping_add(0x1234_5678)).collect();
        let mut os = vec![0i32; n];
        let g: FI32 = i32sar;
        unsafe { g(s.as_ptr(), os.as_mut_ptr(), n) };
        h1[2] = ck(&os.iter().map(|x| *x as i64 as u64).collect::<Vec<_>>());
        let b: Vec<u64> = (0..n).map(|i| (i as u64).wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(0xdead_beef)).collect();
        let mut ob = vec![0u64; n];
        let hs: [F64; 2] = [u64shl, u64shr];
        let mut h2 = [0u64; 2];
        for (k, f) in hs.iter().enumerate() {
            unsafe { f(b.as_ptr(), ob.as_mut_ptr(), n) };
            h2[k] = ck(&ob);
        }
        println!("n={} {} {} {} {} {}", n, h1[0], h1[1], h1[2], h2[0], h2[1]);
    }
}
