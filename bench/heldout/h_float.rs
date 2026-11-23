// Held-out floating point: runtime-sized sin/cos workload.
fn main() {
    let n: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let seed: f64 = std::env::args().nth(2).unwrap().parse().unwrap();
    let mut acc = seed;
    for i in 0..n {
        let x = i as f64 * 0.0001 + seed;
        acc += x.sin() * x.cos();
    }
    println!("{n} {}", acc as u64);
}
