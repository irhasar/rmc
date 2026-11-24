// Held-out calls: recursive work plus a runtime-sized outer loop.
#[inline(never)]
fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

#[inline(never)]
fn mix(x: u64) -> u64 {
    x.wrapping_mul(0x9e3779b97f4a7c15).rotate_left(17) ^ (x >> 7)
}

fn main() {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10000);
    let seed: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let mut acc = seed;
    for i in 0..n {
        acc ^= mix(i ^ acc);
        if i % 4096 == 0 {
            acc = acc.wrapping_add(fib(20));
        }
    }
    println!("{n} {acc}");
}
