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
}
