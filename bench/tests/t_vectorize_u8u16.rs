//! u8/u16 lanes: `c[i]=a[i] OP b[i]` for OP in {+, -, ^, &, |} on byte and word
//! arrays, in address-taken fns (per-function paddb/psubb/pxor/pand/por/paddw/
//! psubw). Exercises full/remainder/sub-vector/zero-trip.
macro_rules! bin {
    ($name:ident, $t:ty, $op:tt) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(pa: *const $t, pb: *const $t, pc: *mut $t, n: usize) {
            let mut i = 0usize;
            while i < n {
                *pc.add(i) = (*pa.add(i)) $op (*pb.add(i));
                i += 1;
            }
        }
    };
}
bin!(u8add, u8, +);
bin!(u8sub, u8, -);
bin!(u8xor, u8, ^);
bin!(u8and, u8, &);
bin!(u8or, u8, |);
bin!(u16add, u16, +);
bin!(u16sub, u16, -);
bin!(u16xor, u16, ^);
bin!(u16and, u16, &);
bin!(u16or, u16, |);
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
        let a: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(7) ^ 0x34).collect();
        let b: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0u8; n];
        let fs: [unsafe extern "C" fn(*const u8, *const u8, *mut u8, usize); 5] =
            [u8add, u8sub, u8xor, u8and, u8or];
        let mut hs = [0u64; 5];
        for (k, f) in fs.iter().enumerate() {
            unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
            hs[k] = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        let q: Vec<u16> = (0..n).map(|i| (i as u16).wrapping_mul(7)).collect();
        let r: Vec<u16> = (0..n).map(|i| (i as u16).wrapping_mul(13)).collect();
        let mut s = vec![0u16; n];
        let gs: [unsafe extern "C" fn(*const u16, *const u16, *mut u16, usize); 5] =
            [u16add, u16sub, u16xor, u16and, u16or];
        let mut hq = [0u64; 5];
        for (k, f) in gs.iter().enumerate() {
            unsafe { f(q.as_ptr(), r.as_ptr(), s.as_mut_ptr(), n) };
            hq[k] = ck(&s.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        println!(
            "n={} {} {} {} {} {} {} {} {} {} {}",
            n, hs[0], hs[1], hs[2], hs[3], hs[4], hq[0], hq[1], hq[2], hq[3], hq[4]
        );
    }
}
