//! Bitwise map: `c[i]=a[i]&b[i]` and `c[i]=a[i]|b[i]` in address-taken fns
//! (per-function `pand`/`por`). Exercises full (1000), remainder (999), sub-vector
//! (2) and zero-trip (0); outputs must equal LLVM.
#[no_mangle]
pub unsafe extern "C" fn bandfn(pa: *const u32, pb: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) & (*pb.add(i));
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn borfn(pa: *const u32, pb: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)) | (*pb.add(i));
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
        let f: unsafe extern "C" fn(*const u32, *const u32, *mut u32, usize) = bandfn;
        unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        let h1 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let f: unsafe extern "C" fn(*const u32, *const u32, *mut u32, usize) = borfn;
        unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        let h2 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        println!("n={} and={} or={}", n, h1, h2);
    }
}
