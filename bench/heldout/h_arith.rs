// Held-out arithmetic: runtime trip count, no constant folding.
fn main() {
    let n: u64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let seed: u64 = std::env::args().nth(2).unwrap().parse().unwrap();
    let mut acc = seed | 1;
    let mut z = 0u64;
    for i in 0..n {
        acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i | 1);
        z ^= acc.rotate_left((i & 31) as u32);
        z = z.wrapping_add((acc as f64).sqrt() as u64);
    }
    println!("{n} {z}");
}
