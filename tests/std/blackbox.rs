// `std::hint::black_box` on one-word values. It inlines to the
// `core::intrinsics::black_box::<T>` intrinsic, which the MIR back end did not
// know: every program that used it to keep a benchmark input opaque - which
// includes this repository's own `bench/rust/*.rs` - was rejected.
use std::hint::black_box;

fn main() {
    let n: i64 = black_box(100_000);
    println!("{}", n * 3 - 1);
    let b: u8 = black_box(200u8);
    println!("{}", b as i64 + 1);
    let f: bool = black_box(n > 5);
    println!("{}", f);
    let x = 42i64;
    let r: &i64 = black_box(&x);
    println!("{}", *r + 1);
    let mut acc: i64 = 0;
    for i in 0..black_box(10i64) {
        acc += black_box(i);
    }
    println!("{}", acc);
}
