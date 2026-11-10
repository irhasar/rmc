#[derive(Copy, Clone)]
struct S { a: i64, b: i64 }
fn make_s(a: i64) -> S { S { a: a, b: a + 1 } }
fn first(n: i64, p: &S) -> i64 { n }

fn main() {
    let s: S = S { a: 1, b: 2 };
    println!("{}", first(3, &s));
    println!("{}", first(7, &{ s }));
    println!("{}", first(9, &make_s(5)));
}
