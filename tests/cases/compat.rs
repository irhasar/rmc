// `use` imports (skipped by rmc),
// `std::process::exit` and `std::hint::black_box`, `assert!`/`assert_eq!`/
// `assert_ne!` (passing), and `panic!` on a path that is not taken.
use std::process;

fn twice(x: i64) -> i64 { std::hint::black_box(x) * 2 }

fn main() {
    let n = twice(21);
    assert!(n == 42);
    assert_eq!(n, 42);
    assert_ne!(n, 41);
    if n == 0 {
        panic!("unreachable");
    }
    println!("{}", n);
    if n == 42 {
        process::exit(0);
    }
    println!("not reached");
}
