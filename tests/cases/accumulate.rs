fn main() {
    let n: i64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let n = std::hint::black_box(n);
    let xs: [i64; 4] = [3, 5, 7, 11];
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        acc = acc + xs[0] + xs[1] + xs[2] + xs[3];
        acc = acc + i + 2;
        acc = acc - i + xs[1] - 1;
        i = i + 1;
    }
    println!("{}", acc);
}
