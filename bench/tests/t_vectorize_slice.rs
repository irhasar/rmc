//! Bounds-checked slice loops: `for i in 0..n { c[i] = a[i] + b[i] }` where the
//! indexing inserts `i < len` checks that branch to a panicking block. The
//! vector loop is clamped to `min(n, len...)` at runtime (no checks inside the
//! vector body); the original checked scalar loop still runs the tail, so a
//! short destination still panics at the exact index.
#[inline(never)]
#[no_mangle]
pub fn slice_map1(pa: &[u32], pc: &mut [u32]) {
    for i in 0..pa.len() {
        pc[i] = pa[i] + 1;
    }
}
#[inline(never)]
#[no_mangle]
pub fn slice_map2(pa: &[u32], pb: &[u32], pc: &mut [u32]) {
    let n = pa.len().min(pb.len());
    for i in 0..n {
        pc[i] = pa[i] + pb[i];
    }
}
#[inline(never)]
#[no_mangle]
pub fn slice_param(pa: &[u32], pc: &mut [u32], n: usize) {
    for i in 0..n {
        pc[i] = pa[i] + 1;
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
    if std::env::args().nth(1).as_deref() == Some("panic") {
        let a: Vec<u32> = (0..8).map(|i| i as u32).collect();
        let mut c = vec![0u32; 6];
        let f: fn(&[u32], &mut [u32]) = std::hint::black_box(slice_map1);
        f(&a, &mut c);
        println!("unreachable {}", c.iter().sum::<u32>());
        return;
    }
    for &raw in &[1000usize, 999, 17, 15, 1, 0] {
        let n = std::hint::black_box(raw);
        let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(2654435761)).collect();
        let b: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(40503)).collect();
        let mut c = vec![0u32; n];
        let f: fn(&[u32], &mut [u32]) = std::hint::black_box(slice_map1);
        f(&a, &mut c);
        let h1 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let g: fn(&[u32], &[u32], &mut [u32]) = std::hint::black_box(slice_map2);
        c.iter_mut().for_each(|x| *x = 0);
        g(&a, &b, &mut c);
        let h2 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let k: fn(&[u32], &mut [u32], usize) = std::hint::black_box(slice_param);
        c.iter_mut().for_each(|x| *x = 0);
        k(&a, &mut c, n);
        let h3 = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        println!("n={} {} {} {}", n, h1, h2, h3);
    }
}
