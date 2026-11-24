// Held-out floating point: runtime-sized sin/cos workload.

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
    let seed: f64 = arg(2, 7.0);
    let mut acc = seed;
    for i in 0..n {
        let x = i as f64 * 0.0001 + seed;
        acc += x.sin() * x.cos();
    }
    println!("{n} {}", acc as u64);
}
