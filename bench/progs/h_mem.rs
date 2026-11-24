// Held-out memory: runtime-sized map, sort and reduction.

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
use std::collections::HashMap;

fn main() {
    let n: usize = arg(1, 10000);
    let seed: u64 = arg(2, 7);
    let mut map: HashMap<u64, Vec<u64>> = HashMap::new();
    for i in 0..n {
        let key = (i as u64).wrapping_mul(seed) % 4096;
        map.entry(key)
            .or_default()
            .push((i as u64).wrapping_mul(2654435761));
    }
    let mut values: Vec<u64> = map.values().flatten().copied().collect();
    values.sort_unstable();
    let sum: u64 = values.iter().step_by(7).fold(0, |a, &b| a.wrapping_add(b));
    println!("{} {} {}", n, values.len(), sum);
}
