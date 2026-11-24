//! Integer sum-reduction vectorization: `acc += a[i]`. Address-taken `usum`
//! (u32, paddd) and `qsum` (u64, paddq). per_vec 4 / 2, so exercise full,
//! remainder, sub-vector and zero-trip; sums must equal LLVM exactly.
#[no_mangle]
pub unsafe extern "C" fn usum(pa: *const u32, n: usize) -> u32 {
    let mut acc = 0u32;
    let mut i = 0usize;
    while i < n {
        acc = acc.wrapping_add(*pa.add(i));
        i += 1;
    }
    acc
}
#[no_mangle]
pub unsafe extern "C" fn qsum(pa: *const u64, n: usize) -> u64 {
    let mut acc = 0u64;
    let mut i = 0usize;
    while i < n {
        acc = acc.wrapping_add(*pa.add(i));
        i += 1;
    }
    acc
}
fn main() {
    for &raw in &[1000usize, 999, 5, 3, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let q: Vec<u64> = (0..n).map(|i| (i as u64).wrapping_mul(13) ^ 0x99).collect();
        let f: unsafe extern "C" fn(*const u32, usize) -> u32 = usum;
        let g: unsafe extern "C" fn(*const u64, usize) -> u64 = qsum;
        let s1 = unsafe { f(a.as_ptr(), n) };
        let s2 = unsafe { g(q.as_ptr(), n) };
        println!("n={} u32={} u64={}", n, s1, s2);
    }
}
