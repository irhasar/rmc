// Held-out branch/reduction: runtime threshold, data-dependent diamond.
fn main() {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10000);
    let threshold: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let mut a = threshold / 2;
    let mut lo = u64::MAX;
    let mut hi = 0u64;
    for i in 0..n {
        if a > threshold {
            hi = hi.max(a);
        } else {
            lo = lo.min(a);
        }
        a = a.wrapping_mul(2862933555777941757).wrapping_add(i | 1);
    }
    println!("{n} {lo} {hi}");
}
