//! u8 unsigned min/max and i16 signed min/max (`pminub`/`pmaxub`/`pminsw`/
//! `pmaxsw`, all SSE2), address-taken fns. `std` min/max lowers to a split body
//! (loads in one block, op+store in the next), exercising the multi-block body
//! matcher. Full/remainder/sub-vector/zero-trip.
#[no_mangle]
pub unsafe extern "C" fn u8min(pa: *const u8, pb: *const u8, pc: *mut u8, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).min(*pb.add(i));
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn u8max(pa: *const u8, pb: *const u8, pc: *mut u8, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).max(*pb.add(i));
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn i16min(pa: *const i16, pb: *const i16, pc: *mut i16, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).min(*pb.add(i));
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn i16max(pa: *const i16, pb: *const i16, pc: *mut i16, n: usize) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).max(*pb.add(i));
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
    for &raw in &[1000usize, 999, 17, 15, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(37) ^ 0x5a).collect();
        let b: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(91) ^ 0x21).collect();
        let mut c = vec![0u8; n];
        let fs: [unsafe extern "C" fn(*const u8, *const u8, *mut u8, usize); 2] = [u8min, u8max];
        let mut hs = [0u64; 2];
        for (k, f) in fs.iter().enumerate() {
            unsafe { f(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
            hs[k] = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        }
        let q: Vec<i16> = (0..n).map(|i| (i as i16).wrapping_mul(311)).collect();
        let r: Vec<i16> = (0..n).map(|i| (i as i16).wrapping_mul(-173)).collect();
        let mut s = vec![0i16; n];
        let gs: [unsafe extern "C" fn(*const i16, *const i16, *mut i16, usize); 2] = [i16min, i16max];
        let mut hq = [0u64; 2];
        for (k, f) in gs.iter().enumerate() {
            unsafe { f(q.as_ptr(), r.as_ptr(), s.as_mut_ptr(), n) };
            hq[k] = ck(&s.iter().map(|x| *x as i64 as u64).collect::<Vec<_>>());
        }
        println!("n={} {} {} {} {}", n, hs[0], hs[1], hq[0], hq[1]);
    }
}
