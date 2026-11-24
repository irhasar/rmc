// Held-out memory: runtime-sized map, sort and reduction.
use std::collections::HashMap;

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10000);
    let seed: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let mut map: HashMap<u64, Vec<u64>> = HashMap::new();
    for i in 0..n {
        let key = (i as u64).wrapping_mul(seed) % 4096;
        map.entry(key).or_default().push((i as u64).wrapping_mul(2654435761));
    }
    let mut values: Vec<u64> = map.values().flatten().copied().collect();
    values.sort_unstable();
    let sum: u64 = values.iter().step_by(7).fold(0, |a, &b| a.wrapping_add(b));
    println!("{} {} {}", n, values.len(), sum);
}
