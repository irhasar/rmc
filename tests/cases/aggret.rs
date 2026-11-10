#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct A { v: i64 }
#[derive(Copy, Clone)]
struct B { a: A }
#[derive(Copy, Clone)]
struct Q { a: [i64; 3] }

fn mk(a: i64) -> P { P { x: a, y: a * 2 } }
fn mk_b() -> B { B { a: A { v: 77 } } }
fn mk_q() -> Q { Q { a: [10, 20, 30] } }

fn main() {
    println!("{}", mk(5).y);
    println!("{}", mk(5).x);
    println!("{}", mk_b().a.v);
    println!("{}", mk_q().a[1]);
    println!("{}", (P { x: 1, y: 2 }).y);
    println!("{}", { P { x: 3, y: 4 } }.y);
    println!("{}", (if true { P { x: 5, y: 6 } } else { P { x: 7, y: 8 } }).y);
    println!("{}", (Q { a: [4, 5, 6] }).a[2]);
}
