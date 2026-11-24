//! Guard rails for the vectorizer beyond the plain map/reduce shapes:
//! - a reduction whose header block stores a counter must stay scalar (the
//!   vector prefix would skip those header executions);
//! - volatile byte accesses must not be widened into SIMD (they are observable
//!   one byte at a time, and rakit's IR has no volatile bit).
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn sum_ctr(pa: *const u32, ctr: *mut u32, n: usize) -> u32 {
    let mut s = 0u32;
    let mut i = 0usize;
    let mut c = 0u32;
    while {
        *ctr = c;
        c += 1;
        i < n
    } {
        s = s.wrapping_add(*pa.add(i));
        i += 1;
    }
    s
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn mul2(pa: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_mul(2);
        i += 1;
    }
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn mul8(pa: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_mul(8);
        i += 1;
    }
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol_loop(pa: *const u8, pc: *mut u8, n: usize) {
    let mut i = 0usize;
    while i < n {
        let v = core::ptr::read_volatile(pa.add(i));
        core::ptr::write_volatile(pc.add(i), v ^ 0x5a);
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
    for &raw in &[1000usize, 4, 3, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u32> = (0..n).map(|i| i as u32).collect();
        let mut ctr = 0u32;
        let f: unsafe extern "C" fn(*const u32, *mut u32, usize) -> u32 = sum_ctr;
        let s = unsafe { f(a.as_ptr(), &mut ctr, n) };
        let ab: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37)).collect();
        let mut ob = vec![0u8; n];
        let g: unsafe extern "C" fn(*const u8, *mut u8, usize) = vol_loop;
        unsafe { g(ab.as_ptr(), ob.as_mut_ptr(), n) };
        let mut o2 = vec![0u32; n];
        let m2: unsafe extern "C" fn(*const u32, *mut u32, usize) = mul2;
        unsafe { m2(a.as_ptr(), o2.as_mut_ptr(), n) };
        let h2 = ck(&o2.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let m8: unsafe extern "C" fn(*const u32, *mut u32, usize) = mul8;
        unsafe { m8(a.as_ptr(), o2.as_mut_ptr(), n) };
        let h8 = ck(&o2.iter().map(|x| *x as u64).collect::<Vec<_>>());
        println!(
            "n={} sum={} ctr={} vol={} mul2={} mul8={}",
            n,
            s,
            ctr,
            ck(&ob.iter().map(|x| *x as u64).collect::<Vec<_>>()),
            h2,
            h8
        );
    }
}
