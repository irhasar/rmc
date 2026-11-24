// Held-out branch/reduction: runtime branch bias, data-dependent diamond.

fn arg<T: std::str::FromStr>(index: usize, fallback: T) -> T {
    match std::env::args().nth(index) {
        None => fallback,
        Some(raw) => raw.parse().unwrap_or_else(|_| {
            eprintln!(
                "invalid argument {index}: {raw:?} (expected {})",
                std::any::type_name::<T>()
            );
            std::process::exit(2)
        }),
    }
}
fn main() {
    let n: u64 = arg(1, 10000);
    let threshold: u64 = arg(2, 7);
    // Scale the small held-out thresholds into the top nibble so the workload
    // keeps taking both arms instead of going high after the first iteration.
    let cut = (threshold & 0xF).wrapping_shl(60);
    let mut a = cut / 2;
    let mut lo = u64::MAX;
    let mut hi = 0u64;
    for i in 0..n {
        if a > cut {
            hi = hi.max(a);
        } else {
            lo = lo.min(a);
        }
        a = a.wrapping_mul(2862933555777941757).wrapping_add(i | 1);
    }
    println!("{n} {lo} {hi}");
}
