//! In-place updates: `a[i] = a[i] OP ...` where the destination base is also a
//! source base. The overlap guard is skipped (same address per index), so these
//! vectorize: u32 add/xor (paddd/pxor), two-source `a[i] += b[i]` (paddd), and
//! f32 `a[i] *= 2.0` (mulps).
#[no_mangle]
pub unsafe extern "C" fn inplace_add1(pa: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pa.add(i) = (*pa.add(i)).wrapping_add(1);
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn inplace_xor(pa: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pa.add(i) = (*pa.add(i)) ^ 0xdeadbeef;
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn inplace_add2(pa: *mut u32, pb: *const u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pa.add(i) = (*pa.add(i)).wrapping_add(*pb.add(i));
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn inplace_fmul(pa: *mut f32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pa.add(i) = (*pa.add(i)) * 2.0;
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
type FU = unsafe extern "C" fn(*mut u32, usize);
type FU2 = unsafe extern "C" fn(*mut u32, *const u32, usize);
type FF = unsafe extern "C" fn(*mut f32, usize);
fn main() {
    for &raw in &[1000usize, 999, 17, 15, 1, 0] {
        let n = std::hint::black_box(raw);
        let base: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(2654435761)).collect();
        let mut hs = [0u64; 4];
        let f: FU = inplace_add1;
        let mut a = base.clone();
        unsafe { f(a.as_mut_ptr(), n) };
        hs[0] = ck(&a.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let f: FU = inplace_xor;
        let mut a = base.clone();
        unsafe { f(a.as_mut_ptr(), n) };
        hs[1] = ck(&a.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let b: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(40503)).collect();
        let f: FU2 = inplace_add2;
        let mut a = base.clone();
        unsafe { f(a.as_mut_ptr(), b.as_ptr(), n) };
        hs[2] = ck(&a.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let fb: Vec<f32> = (0..n).map(|i| (i as f32) * 0.5 - 3.0).collect();
        let f: FF = inplace_fmul;
        let mut af = fb.clone();
        unsafe { f(af.as_mut_ptr(), n) };
        hs[3] = ck(&af.iter().map(|x| x.to_bits() as u64).collect::<Vec<_>>());
        println!("n={} {} {} {} {}", n, hs[0], hs[1], hs[2], hs[3]);
    }
}
