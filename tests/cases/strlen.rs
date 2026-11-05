// rmc's print_i64 writes the same text as `println!`. Rust's `len()` returns
// `usize`, and rmc's only integer type is `i64`, so each length is cast to `i64`
// to line the two up (rmc's `.len()` already yields `i64`).
fn pi(x: i64) {
    println!("{}", x);
}
fn main() {
    // A literal, an empty literal, and a let-bound string.
    pi("hello".len() as i64);
    pi("".len() as i64);
    pi("a".len() as i64);
    let s = "hello, world";
    pi(s.len() as i64);
    // A length used in arithmetic and as a bound.
    let t = "abcd";
    pi(t.len() as i64 + 1);
    pi(t.len() as i64 * 2);
    let mut i = 0;
    let mut sum = 0;
    while i < t.len() as i64 {
        sum = sum + i;
        i = i + 1;
    }
    pi(sum);
    // Array lengths, including one derived from a repeat and one bound to a
    // `let`; both are constants.
    let a = [10, 20, 30];
    pi(a.len() as i64);
    let b = [7; 5];
    pi(b.len() as i64);
    pi(a.len() as i64 + b.len() as i64);
    // A `str` in a variable used as a `while` bound.
    let mut k = 0;
    while k < s.len() as i64 {
        k = k + 1;
    }
    pi(k);
    // `len()` on a string literal directly, in a condition.
    if "abc".len() as i64 == 3 { pi(1); } else { pi(0); }
}
