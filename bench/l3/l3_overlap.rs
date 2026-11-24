// L3 overlap: dst = v[i+2] overlaps sources v[i], v[i+1]. The disjointness guard
// must fall back to the scalar path; hashing all of `v` checks every store.
fn main() {
    let n = 4_000_000usize;
    let mut v: Vec<u32> = (0..n as u32).map(|i| i.wrapping_mul(2654435761)).collect();
    let p = v.as_mut_ptr();
    let m = n - 2;
    for _r in 0..20 {
        let mut i = 0usize;
        while i < m {
            unsafe { *p.add(i + 2) = (*p.add(i) ^ *p.add(i + 1)).wrapping_mul(2654435761); }
            i += 1;
        }
    }
    let mut h = 0u32;
    for &x in &v { h = h.wrapping_mul(16777619) ^ x; }
    println!("{}", h);
}
