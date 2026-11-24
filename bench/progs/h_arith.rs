// Held-out arithmetic: runtime trip count, no constant folding.

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
    let seed: u64 = arg(2, 7);
    let mut acc = seed | 1;
    let mut z = 0u64;
    for i in 0..n {
        acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i | 1);
        z ^= acc.rotate_left((i & 31) as u32);
        z = z.wrapping_add((acc as f64).sqrt() as u64);
    }
    println!("{n} {z}");
}
