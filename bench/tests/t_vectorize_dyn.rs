//! Dynamic trip bound coverage: `n` is opaque (`black_box`), so only the
//! runtime-bound path can vectorize. Exercises the runtime fallbacks: n==0
//! (zero-trip), n=3 (sub-vector), n=1001 (remainder/tail), and an overlapping
//! dst=src+1 (runtime disjointness guard must route to scalar). Each result must
//! match the scalar semantics (LLVM).
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
#[inline(never)]
unsafe fn kern(a: *const u32, b: *const u32, c: *mut u32, n: usize) {
    let mut i = 0usize;
    while i < n {
        *c.add(i) = (*a.add(i) ^ *b.add(i)).wrapping_mul(2654435761);
        i += 1;
    }
}
fn data(n: usize) -> (Vec<u32>, Vec<u32>) {
    let a = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let b = (0..n).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
    (a, b)
}
fn main() {
    // n = 1000: full vector path.
    {
        let n = std::hint::black_box(1000usize);
        let (a, b) = data(n);
        let mut c = vec![0u32; n];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        println!("dyn {} n={}", ck(&c), n);
    }
    // n = 0: zero-trip, destination must stay untouched.
    {
        let n = std::hint::black_box(0usize);
        let (a, b) = data(4);
        let mut c = vec![0xDEAD_BEEFu32; 4];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        println!("zero {} unchanged={}", ck(&c), c.iter().all(|&x| x == 0xDEAD_BEEF));
    }
    // n = 3: sub-vector, c[3] sentinel must stay untouched.
    {
        let n = std::hint::black_box(3usize);
        let (a, b) = data(4);
        let mut c = vec![0u32; 4];
        c[3] = 0xDEAD_BEEF;
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        println!("small {} tail4={}", ck(&c[..n]), c[3] == 0xDEAD_BEEF);
    }
    // n = 1001: vector prefix + scalar remainder.
    {
        let n = std::hint::black_box(1001usize);
        let (a, b) = data(n);
        let mut c = vec![0u32; n];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), n) };
        println!("rem {}", ck(&c));
    }
    // Overlap hazard at runtime: dst = src + 1 feeds a later read.
    {
        let n = std::hint::black_box(1000usize);
        let (a, b) = data(n + 1);
        let mut a = a;
        let p = a.as_mut_ptr();
        unsafe { kern(p, b.as_ptr(), p.add(1), n) };
        println!("overlap {}", ck(&a));
    }
}
