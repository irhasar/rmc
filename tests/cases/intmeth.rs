// Built-in `i64` methods: `abs`, `min`, `max`, `signum`.
const MIN: i64 = -9223372036854775808;

fn pick(x: i64, y: i64) -> i64 {
    return x.min(y) + x.max(y);
}

fn main() {
    let a: i64 = -7;
    let b: i64 = 3;
    println!("{}", a.abs());
    println!("{}", b.abs());
    println!("{}", a.min(b));
    println!("{}", a.max(b));
    println!("{}", b.min(a));
    println!("{}", b.max(a));
    println!("{}", a.signum());
    println!("{}", b.signum());
    let z: i64 = 0;
    println!("{}", z.signum());
    // A tie returns the receiver, for both `min` and `max`.
    let c: i64 = 4;
    println!("{}", c.min(4));
    println!("{}", c.max(4));
    // `i64::MIN`: `abs` maps to itself, `signum` is -1.
    let m: i64 = -9223372036854775808;
    println!("{}", m.abs());
    println!("{}", m.signum());
    // The same through a `const`, which the parser folds: `abs` must not be
    // rejected as a compile-time neg overflow.
    println!("{}", MIN.abs());
    println!("{}", MIN.signum());
    println!("{}", MIN.min(0));
    println!("{}", MIN.max(0));
    // In a helper function, and chained.
    println!("{}", pick(-5, 9));
    println!("{}", a.abs().min(2));
    println!("{}", a.abs().max(100).signum());
    // Through a reference receiver, which auto-derefs.
    let r = &a;
    println!("{}", r.abs());
    // In a loop.
    let mut s = 0;
    let mut i: i64 = -3;
    while i < 3 {
        s += i.abs();
        i += 1;
    }
    println!("{}", s);

    // `div_euclid` / `rem_euclid`: the quotient rounded toward the
    // mathematical value and the remainder in `[0, |y|)`. `i64::MIN`
    // as the divisor must not overflow the `r - MIN` arm, which never
    // runs.
    println!("{}", a.div_euclid(b));
    println!("{}", a.rem_euclid(b));
    println!("{}", b.div_euclid(a));
    println!("{}", b.rem_euclid(a));
    println!("{}", a.div_euclid(-b));
    println!("{}", a.rem_euclid(-b));
    println!("{}", b.div_euclid(-a));
    println!("{}", b.rem_euclid(-a));
    println!("{}", m.div_euclid(b));
    println!("{}", m.rem_euclid(b));
    println!("{}", a.rem_euclid(m));
    println!("{}", a.div_euclid(m));
    println!("{}", b.rem_euclid(m));
    println!("{}", b.div_euclid(m));
    println!("{}", a.div_euclid(MIN));
    println!("{}", a.rem_euclid(MIN));
    println!("{}", r.div_euclid(b));
    println!("{}", r.rem_euclid(b));
    // `is_positive` / `is_negative` yield `bool`; `0` is neither, `i64::MIN`
    // is negative and not positive.
    println!("{}", a.is_negative() as i64);
    println!("{}", a.is_positive() as i64);
    println!("{}", b.is_positive() as i64);
    println!("{}", z.is_positive() as i64);
    println!("{}", z.is_negative() as i64);
    println!("{}", m.is_negative() as i64);
    println!("{}", m.is_positive() as i64);
    println!("{}", MIN.is_negative() as i64);
    println!("{}", r.is_positive() as i64);
    if a.is_negative() {
        println!("{}", 1);
    } else {
        println!("{}", 0);
    }

    // `clamp`: `x` bounded to `[lo, hi]`, a tie with either bound returning
    // that bound. `min > max` is a run-time panic, so every call here has
    // `lo <= hi`; `MIN` as a bound must not overflow the comparisons.
    println!("{}", a.clamp(-10, 10));
    println!("{}", a.clamp(-5, 10));
    println!("{}", a.clamp(-10, -8));
    println!("{}", a.clamp(a, b));
    println!("{}", b.clamp(a, b));
    println!("{}", z.clamp(a, b));
    println!("{}", MIN.clamp(MIN, b));
    println!("{}", b.clamp(MIN, MIN));
    println!("{}", a.clamp(b, b));
    println!("{}", pick(-5, 9).clamp(0, 100));
    println!("{}", a.abs().clamp(0, 3));

    // `count_ones` / `leading_zeros` / `trailing_zeros`: exact bit counts for
    // every input, including the `i64` extremes, with no trap under
    // `-C overflow-checks=on` (the helpers never negate or decrement).
    println!("{}", z.count_ones());
    println!("{}", z.leading_zeros());
    println!("{}", z.trailing_zeros());
    println!("{}", a.count_ones());
    println!("{}", a.leading_zeros());
    println!("{}", a.trailing_zeros());
    println!("{}", MIN.count_ones());
    println!("{}", MIN.leading_zeros());
    println!("{}", MIN.trailing_zeros());
    let mx: i64 = 9223372036854775807;
    println!("{}", mx.count_ones());
    println!("{}", mx.leading_zeros());
    println!("{}", mx.trailing_zeros());
    let ones: i64 = -1;
    println!("{}", ones.count_ones());
    println!("{}", ones.leading_zeros());
    println!("{}", ones.trailing_zeros());
    // Through a reference receiver, and chained onto another method.
    println!("{}", r.count_ones());
    println!("{}", a.abs().trailing_zeros());
    println!("{}", mx.count_ones().trailing_zeros());

    // `leading_ones` / `trailing_ones`: the count of leading/trailing set bits.
    // They reuse the zero counts on `!x`, so the `i64` extremes are exact too
    // (`MIN.leading_ones()` is 1, `-1.leading_ones()` is 64).
    println!("{}", z.leading_ones());
    println!("{}", z.trailing_ones());
    println!("{}", a.leading_ones());
    println!("{}", a.trailing_ones());
    println!("{}", MIN.leading_ones());
    println!("{}", MIN.trailing_ones());
    println!("{}", mx.leading_ones());
    println!("{}", mx.trailing_ones());
    println!("{}", ones.leading_ones());
    println!("{}", ones.trailing_ones());
    println!("{}", r.leading_ones());

    // `swap_bytes` / `reverse_bits`: the byte and bit order reversed. A single
    // `bswap` and a per-bit loop, exact for every input.
    println!("{}", z.swap_bytes());
    println!("{}", z.reverse_bits());
    println!("{}", a.swap_bytes());
    println!("{}", a.reverse_bits());
    println!("{}", MIN.swap_bytes());
    println!("{}", MIN.reverse_bits());
    println!("{}", mx.swap_bytes());
    println!("{}", mx.reverse_bits());
    println!("{}", ones.swap_bytes());
    println!("{}", ones.reverse_bits());
    println!("{}", r.swap_bytes());
    // Chained, and on a call result.
    println!("{}", a.swap_bytes().reverse_bits());
    println!("{}", pick(-5, 9).reverse_bits());

    // `count_zeros`: the mirror of `count_ones`, `count_ones(!x)`.
    println!("{}", z.count_zeros());
    println!("{}", a.count_zeros());
    println!("{}", MIN.count_zeros());
    println!("{}", mx.count_zeros());
    println!("{}", ones.count_zeros());

    // `rotate_left` / `rotate_right`: the count is taken mod 64, so 64 is a full
    // turn (the identity) and 65 wraps to 1. Exact for every count.
    println!("{}", z.rotate_left(1));
    println!("{}", a.rotate_left(3));
    println!("{}", a.rotate_right(3));
    println!("{}", MIN.rotate_left(1));
    println!("{}", mx.rotate_left(1));
    println!("{}", ones.rotate_left(5));
    println!("{}", ones.rotate_right(5));
    println!("{}", a.rotate_left(0));
    println!("{}", a.rotate_left(64));
    println!("{}", a.rotate_left(65));
    println!("{}", a.rotate_right(64));
    println!("{}", a.rotate_left(63));
    println!("{}", MIN.rotate_left(64));
    println!("{}", mx.rotate_right(1));
    println!("{}", a.rotate_left(1).count_ones());
    println!("{}", pick(1, 2).rotate_right(1));
}
