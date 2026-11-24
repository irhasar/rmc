//! The post-egraph vectorizer must not change observable behaviour. Unit-stride
//! disjoint loops vectorize; strided loops, zero-trip loops, overlapping
//! buffers (a store feeding a later iteration's read), in-place writes, tails
//! and narrow (u8) loops must all still match the scalar semantics.
fn ck(v: &[u32]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x as u64).wrapping_mul(1099511628211);
    }
    h
}
unsafe fn kern(a: *const u32, b: *const u32, c: *mut u32, n: usize, k: u32) {
    let mut i = 0usize;
    while i < n {
        *c.add(i) = (*a.add(i) ^ *b.add(i)).wrapping_mul(k);
        i += 1;
    }
}
unsafe fn kern2(a: *const u32, b: *const u32, c: *mut u32, n: usize, k: u32) {
    let mut i = 0usize;
    while i < n {
        *c.add(i) = (*a.add(i) ^ *b.add(i)).wrapping_mul(k);
        i += 2;
    }
}
unsafe fn kern8(a: *const u8, b: *const u8, c: *mut u8, n: usize, k: u8) {
    let mut i = 0usize;
    while i < n {
        *c.add(i) = (*a.add(i) ^ *b.add(i)).wrapping_mul(k);
        i += 1;
    }
}
fn main() {
    const K: u32 = 2654435761;
    {
        const N: usize = 1000;
        let a: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0u32; N];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), N, K) };
        println!("unit {}", ck(&c));
    }
    {
        const N: usize = 1000;
        let a: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0xDEAD_BEEFu32; N];
        unsafe { kern2(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), N, K) };
        println!("strided2 {}", ck(&c));
    }
    {
        // Zero-trip (n == 0): the matcher rejects zero bounds, so this is the
        // scalar path; it must leave the destination untouched. The generated
        // SIMD guard with zero vector iterations is covered by t_vectorize_guard.
        let a = vec![7u32; 4];
        let b = vec![9u32; 4];
        let mut c = vec![0xDEAD_BEEFu32; 4];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), 0, K) };
        println!("zerotrip {} unchanged={}", ck(&c), c.iter().all(|&x| x == 0xDEAD_BEEF));
    }
    {
        // Sub-vector trip (n = 2): only the scalar tail runs.
        const N: usize = 2;
        let a: Vec<u32> = (0..N).map(|i| (i as u32) + 5).collect();
        let b: Vec<u32> = (0..N).map(|i| (i as u32) + 9).collect();
        let mut c = vec![0u32; N];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), N, K) };
        println!("small {}", ck(&c));
    }
    {
        // Overwrite hazard: dst = src + 1, so a store feeds a later read. This
        // differs from the scalar result if the overlap guard is missing.
        const N: usize = 1000;
        let mut a: Vec<u32> = (0..N + 1).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
        unsafe { kern(a.as_ptr(), b.as_ptr(), a.as_mut_ptr().add(1), N, K) };
        println!("overlap {}", ck(&a));
    }
    {
        // In-place: dst aliases a source exactly.
        const N: usize = 1000;
        let mut a: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
        let p = a.as_mut_ptr();
        unsafe { kern(p, b.as_ptr(), p, N, K) };
        println!("inplace {}", ck(&a));
    }
    {
        const N: usize = 1001;
        let a: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
        let b: Vec<u32> = (0..N).map(|i| (i as u32).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0u32; N];
        unsafe { kern(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), N, K) };
        println!("tail {}", ck(&c));
    }
    {
        const N: usize = 1000;
        let a: Vec<u8> = (0..N).map(|i| (i as u8).wrapping_mul(7) ^ 0x12).collect();
        let b: Vec<u8> = (0..N).map(|i| (i as u8).wrapping_mul(13) ^ 0x99).collect();
        let mut c = vec![0u8; N];
        unsafe { kern8(a.as_ptr(), b.as_ptr(), c.as_mut_ptr(), N, 201) };
        let cu: Vec<u32> = c.iter().map(|&x| x as u32).collect();
        println!("u8 {}", ck(&cu));
    }
}
