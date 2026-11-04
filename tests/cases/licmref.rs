struct S {
    a: i64,
    b: i64,
}
fn inc(s: &mut S) -> i64 {
    s.a = s.a + 1;
    s.a
}
fn main() {
    let mut s = S { a: 1, b: 2 };
    let mut i: i64 = 0;
    let mut t: i64 = 0;
    while i < 5 {
        t = t + inc(&mut s) + s.b;
        i = i + 1;
    }
    println!("{}", t);
    println!("{}", s.a);
}
