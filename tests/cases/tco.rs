fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

fn fact(n: i64) -> i64 {
    if n < 2 {
        return 1;
    }
    fact(n - 1) * n
}

fn main() {
    let n: i64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let n = std::hint::black_box(n);
    println!("{}", fib(n));
    println!("{}", fact(n));
}
