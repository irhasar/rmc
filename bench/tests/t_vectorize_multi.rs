//! Two independent eligible loops in ONE function. The function is address-taken
//! (`let f = two_loops;`) so it stays a distinct symbol and is not inlined, which
//! lets the regression harness count `pmuludq` *inside `two_loops`* rather than
//! binary-wide (a packed multiply elsewhere could otherwise mask a scalar loop).
//! Each loop lowers to 2 `pmuludq` on SSE2, so a correct build has >= 4 in this
//! function; an early exit after the first match would leave only 2.
#[no_mangle]
pub unsafe extern "C" fn two_loops(
    pa: *const u32, pb: *const u32, pc: *mut u32,
    qa: *const u32, qb: *const u32, qc: *mut u32, n: usize,
) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761);
        i += 1;
    }
    let mut j = 0usize;
    while j < n {
        *qc.add(j) = (*qa.add(j) ^ *qb.add(j)).wrapping_mul(40503);
        j += 1;
    }
}
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    let n = std::hint::black_box(1000usize);
    let a1: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let b1: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
    let mut c1 = vec![0u32; n];
    let a2: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(17) ^ 0xabcd).collect();
    let b2: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(29) ^ 0x55).collect();
    let mut c2 = vec![0u32; n];
    let f: unsafe extern "C" fn(*const u32, *const u32, *mut u32, *const u32, *const u32, *mut u32, usize) =
        two_loops;
    unsafe {
        f(a1.as_ptr(), b1.as_ptr(), c1.as_mut_ptr(), a2.as_ptr(), b2.as_ptr(), c2.as_mut_ptr(), n)
    };
    println!("m1 {} m2 {}", ck(&c1), ck(&c2));
}
