//! Volatile accesses must stay observable: no widening into SIMD, a read whose
//! result is discarded must still happen, and wide values (no single x64 load
//! above 128 bits) must compile by reading supported-size chunks.
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol_u8_loop(pa: *const u8, pc: *mut u8, n: usize) {
    let mut i = 0usize;
    while i < n {
        let v = core::ptr::read_volatile(pa.add(i));
        core::ptr::write_volatile(pc.add(i), v ^ 0x5a);
        i += 1;
    }
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol_discard_u8(pa: *const u8) {
    let _ = core::ptr::read_volatile(pa);
}
#[cfg(target_arch = "x86_64")]
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol256_discard(p: *const core::arch::x86_64::__m256i) {
    let _ = core::ptr::read_volatile(p);
}
#[cfg(target_arch = "x86_64")]
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol256_use(p: *const core::arch::x86_64::__m256i, out: *mut u64) {
    let v: [u64; 4] = core::mem::transmute(core::ptr::read_volatile(p));
    *out.add(0) = v[0];
    *out.add(1) = v[1];
    *out.add(2) = v[2];
    *out.add(3) = v[3];
}
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Blob12 {
    a: u32,
    b: u32,
    c: u32,
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol12_discard(p: *const Blob12) {
    let _ = core::ptr::read_volatile(p);
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol12_use(p: *const Blob12, out: *mut u32) {
    let v: Blob12 = core::ptr::read_volatile(p);
    *out.add(0) = v.a;
    *out.add(1) = v.b;
    *out.add(2) = v.c;
}
fn ck(v: &[u64]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    for &raw in &[1000usize, 17, 1, 0] {
        let n = std::hint::black_box(raw);
        let ab: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37)).collect();
        let mut ob = vec![0u8; n];
        let g: unsafe extern "C" fn(*const u8, *mut u8, usize) = vol_u8_loop;
        unsafe { g(ab.as_ptr(), ob.as_mut_ptr(), n) };
        let one = [7u8; 1];
        let d: unsafe extern "C" fn(*const u8) = std::hint::black_box(vol_discard_u8);
        unsafe { d(one.as_ptr()) };
        println!(
            "n={} loop={}",
            n,
            ck(&ob.iter().map(|x| *x as u64).collect::<Vec<_>>())
        );
    }
    #[cfg(target_arch = "x86_64")]
    {
        let src: [u64; 4] = [
            0x0123456789abcdef,
            0xfedcba9876543210,
            0x1111222233334444,
            0x5555666677778888,
        ];
        let p = src.as_ptr() as *const core::arch::x86_64::__m256i;
        let d: unsafe extern "C" fn(*const core::arch::x86_64::__m256i) =
            std::hint::black_box(vol256_discard);
        unsafe { d(p) };
        let mut out = [0u64; 4];
        let u: unsafe extern "C" fn(*const core::arch::x86_64::__m256i, *mut u64) =
            std::hint::black_box(vol256_use);
        unsafe { u(p, out.as_mut_ptr()) };
        println!("v256 {}", ck(&out));
    }
    let b = Blob12 {
        a: 0xdeadbeef,
        b: 0x12345678,
        c: 0xcafebabe,
    };
    let bp = &b as *const Blob12;
    let d: unsafe extern "C" fn(*const Blob12) = std::hint::black_box(vol12_discard);
    unsafe { d(bp) };
    let mut o2 = [0u32; 3];
    let u: unsafe extern "C" fn(*const Blob12, *mut u32) = std::hint::black_box(vol12_use);
    unsafe { u(bp, o2.as_mut_ptr()) };
    println!("v12 {} {} {}", o2[0], o2[1], o2[2]);
}
