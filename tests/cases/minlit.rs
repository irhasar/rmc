// The `i64::MIN` literal `-9223372036854775808`: the lexer sees the magnitude
// `9223372036854775808`, which exceeds `i64::MAX`, before the sign, so the
// parser folds the `-` and the magnitude into `Int(i64::MIN)` only when they
// are adjacent. This exercises the value in a typed binding, arithmetic, the
// bitwise `!`, a comparison and a `match` pattern.
fn main() {
    let m: i64 = -9223372036854775808;
    println!("{}", m);
    println!("{}", m + 1);
    println!("{}", m / 2);
    println!("{}", !m);
    println!("{}", (m < 0) as i64);
    println!("{}", match m { -9223372036854775808 => 11, _ => 22 });
}
