// The arithmetic methods that lower to a runtime helper: `pow`,
// `wrapping_add`/`wrapping_sub`, and the `saturating_add`/`_sub`/`_mul`
// family. Every line must match `rustc -O`'s output.
const MAX: i64 = 9223372036854775807;
const MIN: i64 = -9223372036854775808;

fn main() {
    // `pow` by binary exponentiation, wrapping on overflow like `rustc -O`.
    println!("{}", 2i64.pow(10));
    println!("{}", 3i64.pow(0));
    println!("{}", 5i64.pow(1));
    println!("{}", (-2i64).pow(3));
    println!("{}", (-2i64).pow(4));
    println!("{}", 2i64.pow(62));
    println!("{}", 2i64.pow(63));
    println!("{}", 2i64.pow(64));
    println!("{}", (-2i64).pow(63));
    println!("{}", 10i64.pow(18));
    // A computed exponent: rustc infers the `u32` argument from the literal.
    let b = 4i64;
    println!("{}", b.pow(2 + 1));

    // `wrapping_add` / `wrapping_sub` are the bare operators.
    println!("{}", MAX.wrapping_add(1));
    println!("{}", MIN.wrapping_sub(1));
    println!("{}", 5i64.wrapping_add(3));
    println!("{}", 5i64.wrapping_sub(3));
    println!("{}", 0i64.wrapping_sub(1));

    // `saturating_*` clamps at the bounds rather than wrapping.
    println!("{}", MAX.saturating_add(1));
    println!("{}", MIN.saturating_add(-1));
    println!("{}", 5i64.saturating_add(3));
    println!("{}", MAX.saturating_sub(-1));
    println!("{}", MIN.saturating_sub(1));
    println!("{}", 5i64.saturating_sub(3));
    println!("{}", MAX.saturating_mul(2));
    println!("{}", MAX.saturating_mul(-2));
    println!("{}", MIN.saturating_mul(2));
    println!("{}", MIN.saturating_mul(-2));
    println!("{}", 100i64.saturating_mul(3));
    println!("{}", (-100i64).saturating_mul(3));

    // Chained, and through a reference receiver (which auto-derefs).
    println!("{}", 2i64.pow(3).saturating_add(100).wrapping_sub(1));
    let r = &b;
    println!("{}", r.pow(4));
    println!("{}", r.saturating_mul(MAX));

    // In a loop, accumulating with saturation.
    let mut s = 0i64;
    let mut i = 1i64;
    while i <= 6 {
        s = s.saturating_add(i.pow(2));
        i += 1;
    }
    println!("{}", s);
}
