// The `args().nth(i)` chain has two distinct failure points: `Option::unwrap`
// fails on a missing argument, `Result::unwrap_or(d)` on a present string that
// is not an integer. They must be tracked separately, so a malformed present
// argument takes the fallback while an absent one still exits. `parse::<i64>()`
// carries a turbofish whose type name is a keyword token.
use std::env;

fn main() {
    let a: i64 = env::args().nth(1).unwrap().parse::<i64>().unwrap_or(42);
    println!("{}", a);
    let b: i64 = env::args()
        .nth(2)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(99);
    println!("{}", b);
    let c: i64 = env::args().nth(3).unwrap().parse().unwrap();
    println!("{}", c);
}
