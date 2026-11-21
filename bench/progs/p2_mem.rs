// memory / allocation / collection heavy
use std::collections::HashMap;
fn main() {
    let mut m: HashMap<u64, Vec<u64>> = HashMap::new();
    for i in 0..300_000u64 {
        m.entry(i % 4096).or_default().push(i.wrapping_mul(2654435761));
    }
    let mut v: Vec<u64> = m.values().flatten().copied().collect();
    v.sort_unstable();
    let s: u64 = v.iter().step_by(7).sum();
    println!("{} {}", v.len(), s);
}
