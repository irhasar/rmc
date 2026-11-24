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
// 32-byte aligned wrapper so the `__m256i` cast below is well defined.
#[cfg(target_arch = "x86_64")]
#[repr(C, align(32))]
pub struct Aligned32(pub [u64; 4]);

// `Blob12` only guarantees 4-byte alignment; placing it at offset 4 keeps the
// address 4-aligned, which must not produce 8-byte loads.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Wrap {
    pub pad: u32,
    pub blob: Blob12,
}

// Large aggregate: the read must use a counted loop of explicit loads, never a
// library copy (which could read the source twice).
#[repr(C)]
pub struct Big512(pub [u64; 64]);

// Five read-to-clear registers: a copy that overlaps its loads would read
// offsets 8..24 twice.
#[repr(C)]
pub struct Big40(pub [u64; 5]);

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Blob12 {
    a: u32,
    b: u32,
    c: u32,
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol40_discard(p: *const Big40) {
    let _ = core::ptr::read_volatile(p);
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol40_use(p: *const Big40, out: *mut u64) {
    let v: Big40 = core::ptr::read_volatile(p);
    *out.add(0) = v.0[0];
    *out.add(1) = v.0[1];
    *out.add(2) = v.0[2];
    *out.add(3) = v.0[3];
    *out.add(4) = v.0[4];
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol512_discard(p: *const Big512) {
    let _ = core::ptr::read_volatile(p);
}
#[inline(never)]
#[no_mangle]
pub unsafe extern "C" fn vol512_use(p: *const Big512, out: *mut u64) {
    let v: Big512 = core::ptr::read_volatile(p);
    *out.add(0) = v.0[0];
    *out.add(1) = v.0[63];
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
        let src = Aligned32([
            0x0123456789abcdef,
            0xfedcba9876543210,
            0x1111222233334444,
            0x5555666677778888,
        ]);
        let p = (&src.0 as *const [u64; 4]) as *const core::arch::x86_64::__m256i;
        let d: unsafe extern "C" fn(*const core::arch::x86_64::__m256i) =
            std::hint::black_box(vol256_discard);
        unsafe { d(p) };
        let mut out = [0u64; 4];
        let u: unsafe extern "C" fn(*const core::arch::x86_64::__m256i, *mut u64) =
            std::hint::black_box(vol256_use);
        unsafe { u(p, out.as_mut_ptr()) };
        println!("v256 {}", ck(&out));
    }
    // Field at offset 4 inside an 8-byte-aligned wrapper: 4-byte aligned only.
    let w = Wrap {
        pad: 0x0badf00d,
        blob: Blob12 {
            a: 0xdeadbeef,
            b: 0x12345678,
            c: 0xcafebabe,
        },
    };
    let bp = &w.blob as *const Blob12;
    let d: unsafe extern "C" fn(*const Blob12) = std::hint::black_box(vol12_discard);
    unsafe { d(bp) };
    let mut o2 = [0u32; 3];
    let u: unsafe extern "C" fn(*const Blob12, *mut u32) = std::hint::black_box(vol12_use);
    unsafe { u(bp, o2.as_mut_ptr()) };
    println!("v12 {} {} {}", o2[0], o2[1], o2[2]);
    let b40 = Big40([
        0x1111111111111111,
        0x2222222222222222,
        0x3333333333333333,
        0x4444444444444444,
        0x5555555555555555,
    ]);
    let b40p = &b40 as *const Big40;
    let d: unsafe extern "C" fn(*const Big40) = std::hint::black_box(vol40_discard);
    unsafe { d(b40p) };
    let mut o40 = [0u64; 5];
    let u: unsafe extern "C" fn(*const Big40, *mut u64) = std::hint::black_box(vol40_use);
    unsafe { u(b40p, o40.as_mut_ptr()) };
    println!("v40 {}", ck(&o40));
    let big = Big512(core::array::from_fn(|i| (i as u64).wrapping_mul(0x9e3779b97f4a7c15)));
    let bigp = &big as *const Big512;
    let d: unsafe extern "C" fn(*const Big512) = std::hint::black_box(vol512_discard);
    unsafe { d(bigp) };
    let mut o3 = [0u64; 2];
    let u: unsafe extern "C" fn(*const Big512, *mut u64) = std::hint::black_box(vol512_use);
    unsafe { u(bigp, o3.as_mut_ptr()) };
    println!("v512 {:x} {:x}", o3[0], o3[1]);
}
