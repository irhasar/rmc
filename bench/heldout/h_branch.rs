// Held-out branch/reduction: runtime threshold, data-dependent diamond.
fn main() {
    let n: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let threshold: u64 = std::env::args().nth(2).unwrap().parse().unwrap();
    let mut a = threshold | 1;
    let mut lo = u64::MAX;
    let mut hi = 0u64;
    for i in 0..n {
        a = a.wrapping_mul(2862933555777941757).wrapping_add(i | 1);
        if a > threshold {
            hi = hi.max(a);
        } else {
            lo = lo.min(a);
        }
    }
    println!("{n} {lo} {hi}");
}
