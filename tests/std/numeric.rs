// Integer `min`/`max` print as `<T as Ord>::min`/`max`, which the back end
// rewrites to a signed or unsigned runtime helper chosen from `T`. Both a
// constant and a runtime operand are covered, plus the widths that pick the
// 32-bit form.
fn main() {
    println!("{}", 3.min(5));
    println!("{}", 3i64.max(5));
    println!("{}", (-3i32).min(-5));
    println!("{}", 200u8.max(100));

    let a: i64 = 17;
    let b: i64 = 4;
    println!("{} {}", a.min(b), a.max(b));

    let c: u32 = 4_000_000_000;
    let d: u32 = 7;
    println!("{} {}", c.min(d), c.max(d));

    // A negative operand must compare signed: unsigned ordering would pick 7.
    println!("{}", (-7i64).max(7));

    // `{}` of a narrow integer loads exactly one byte; an `i8` must sign-extend
    // all the way to 64 bits, or the value prints as a huge unsigned number.
    let e: u8 = 200;
    let f: i8 = -5;
    println!("{} {} {}", e, f, 7u8);
    println!("{:x} {:b} {:o}", 200u8, -5i8, 200u8);
}
