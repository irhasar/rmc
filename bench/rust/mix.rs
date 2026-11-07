use std::env;
fn gcd(a: i64, b: i64) -> i64 {
    let mut x = a;
    let mut y = b;
    while y != 0 {
        let t = x % y;
        x = y;
        y = t;
    }
    x
}
fn main() {
    let n: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20000);
    let n = std::hint::black_box(n);
    let mut sum: i64 = 0;
    let mut i: i64 = 1;
    while i < n {
        sum += gcd(i, 2 * i + 3);
        i += 1;
    }
    println!("{}", sum);
}
