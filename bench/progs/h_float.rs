// Held-out floating point: runtime-sized sin/cos workload.
fn main() {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10000);
    let seed: f64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7.0);
    let mut acc = seed;
    for i in 0..n {
        let x = i as f64 * 0.0001 + seed;
        acc += x.sin() * x.cos();
    }
    println!("{n} {}", acc as u64);
}
