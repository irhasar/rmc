fn add(a: i64, b: i64) -> i64 {
    a + b
}
fn six(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64) -> i64 {
    a * 1 + b * 2 + c * 3 + d * 4 + e * 5 + f * 6
}
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}
fn ack(m: i64, n: i64) -> i64 {
    if m == 0 {
        return n + 1;
    }
    if n == 0 {
        return ack(m - 1, 1);
    }
    ack(m - 1, ack(m, n - 1))
}
fn main() {
    println!("{}", add(2, 3));
    println!("{}", add(add(1, 2), add(3, 4)));
    println!("{}", six(1, 2, 3, 4, 5, 6));
    println!("{}", fib(20));
    println!("{}", ack(2, 3));
    println!("{}", ack(3, 3));
}
