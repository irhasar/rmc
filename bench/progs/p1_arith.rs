// integer/float arithmetic heavy, branchy
fn main() {
    let mut acc: u64 = 1;
    let mut f: f64 = 0.0;
    for i in 0..20_000_000u64 {
        acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i | 1);
        f += (i as f64).sqrt() * 0.5;
        if acc % 7 == 0 { acc ^= f as u64; }
    }
    println!("{} {}", acc, f);
}
