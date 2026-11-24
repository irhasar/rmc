//! Add and sub combines in ONE address-taken function (`ops2`), so the harness
//! can assert `paddd`/`psubd` *inside `ops2`* rather than binary-wide. Add is
//! commutative; sub is not, so this also pins operand order (`a - b`).
#[no_mangle]
pub unsafe extern "C" fn ops2(
    pa: *const u32, pb: *const u32, pc: *mut u32,
    qa: *const u32, qb: *const u32, qc: *mut u32, n: usize,
) {
    let mut i = 0usize;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_add(*pb.add(i));
        i += 1;
    }
    let mut j = 0usize;
    while j < n {
        *qc.add(j) = (*qa.add(j)).wrapping_sub(*qb.add(j));
        j += 1;
    }
}
// `a - b` with the loads emitted in the *opposite* order (b first), so the
// vectorizer must map operands to bases by value, not by load order.
#[no_mangle]
pub unsafe extern "C" fn ops2r(pa: *const u32, pb: *const u32, pc: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        let b = *pb.add(i);
        let a = *pa.add(i);
        *pc.add(i) = a.wrapping_sub(b);
        i += 1;
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
        ops2;
    unsafe {
        f(a1.as_ptr(), b1.as_ptr(), c1.as_mut_ptr(), a2.as_ptr(), b2.as_ptr(), c2.as_mut_ptr(), n)
    };
    let mut c3 = vec![0u32; n];
    let r: unsafe extern "C" fn(*const u32, *const u32, *mut u32, usize) = ops2r;
    unsafe { r(a2.as_ptr(), b2.as_ptr(), c3.as_mut_ptr(), n) };
    println!("add {} sub {} rev {}", ck(&c1), ck(&c2), ck(&c3));
}
