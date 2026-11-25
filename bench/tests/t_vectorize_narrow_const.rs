//! u8/u16 broadcast-constant bodies: `c[i] = a[i] OP K` for OP in {+, ^, &, |}
//! (paddb/pxor/pand/por/paddw). Constant is an `iconst.i8`/`i16` splat; full,
//! remainder, sub-vector and zero-trip cases.
macro_rules! kbin {
    ($name:ident, $t:ty, $op:tt, $k:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(pa: *const $t, pc: *mut $t, n: usize) {
            let mut i = 0usize;
            while i < n {
                *pc.add(i) = (*pa.add(i)) $op $k;
                i += 1;
            }
        }
    };
}
kbin!(u8add, u8, +, 5);
kbin!(u8xor, u8, ^, 0x33);
kbin!(u8and, u8, &, 0x0f);
kbin!(u8or, u8, |, 0x80);
kbin!(u16add, u16, +, 7);
kbin!(u16xor, u16, ^, 0x1234);
kbin!(u16and, u16, &, 0x0ff0);
kbin!(u16or, u16, |, 0x8000);
fn ck(v: &[u64]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    for &raw in &[1000usize, 999, 17, 15, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37) ^ 0x5a).collect();
        let mut c = vec![0u8; n];
        let fs: [unsafe extern "C" fn(*const u8, *mut u8, usize); 4] = [u8add, u8xor, u8and, u8or];
        let mut hs = [0u64; 4];
        for (k, f) in fs.iter().enumerate() {
            unsafe { f(a.as_ptr(), c.as_mut_ptr(), n) };
            hs[k] = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        let q: Vec<u16> = (0..n).map(|i| (i as u16).wrapping_mul(311)).collect();
        let mut s = vec![0u16; n];
        let gs: [unsafe extern "C" fn(*const u16, *mut u16, usize); 4] = [u16add, u16xor, u16and, u16or];
        let mut hq = [0u64; 4];
        for (k, f) in gs.iter().enumerate() {
            unsafe { f(q.as_ptr(), s.as_mut_ptr(), n) };
            hq[k] = ck(&s.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        println!(
            "n={} {} {} {} {} {} {} {} {}",
            n, hs[0], hs[1], hs[2], hs[3], hq[0], hq[1], hq[2], hq[3]
        );
    }
}
