use std::env;
fn tree(n: i64) -> i64 {
    if n <= 1 {
        return n;
    }
    1 + tree(n - 1) + tree(n - 2)
}
fn main() {
    let n: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let n = std::hint::black_box(n);
    println!("{}", tree(n));
}
