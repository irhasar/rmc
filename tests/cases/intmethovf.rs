// `i64::MIN.abs()` negates `i64::MIN`, so under overflow checks it traps, as a
// debug rustc does. The trap prints nothing on stdout, so the twins agree.
const MIN: i64 = -9223372036854775808;

fn main() {
    println!("{}", MIN.abs());
}
