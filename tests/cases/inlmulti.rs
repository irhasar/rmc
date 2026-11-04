fn bump(mut x: i64) -> i64 {
    x = x + 1;
    x + x
}
fn id(x: i64) -> i64 {
    x
}
fn add(a: i64, b: i64) -> i64 {
    a + b
}
fn dbl(x: i64) -> i64 {
    x + x
}
fn quad(x: i64) -> i64 {
    dbl(x) + dbl(x)
}
fn main() {
    let y: i64 = 5;
    println!("{}", bump(y));
    println!("{}", y);
    println!("{}", bump(3 * 2));
    println!("{}", add(id(1), id(2)));
    println!("{}", quad(3));
}
