// Regressions from a code review. Each previously produced a wrong answer.
#[derive(Clone, Copy)]
struct Inner { x: i64, y: i64 }
#[derive(Clone, Copy)]
struct Outer { a: i64, inner: Inner }
#[derive(Clone, Copy)]
struct Big { a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64, i: i64 }

fn make_inner(v: i64) -> Inner { Inner { x: v, y: v + 1 } }
fn fst(a: i64, _b: i64) -> i64 { a }
fn next() -> usize { 1 }

fn main() {
    let mut o: Outer = Outer { a: 1, inner: Inner { x: 0, y: 0 } };
    o.inner = make_inner(5);
    println!("{}", o.a);
    println!("{}", o.inner.x);
    println!("{}", o.inner.y);

    let big: Big = Big { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9 };
    println!("{}", fst(7, { let tmp: Big = big; 2 }));

    let __rmc_ix0: i64 = 10;
    let mut a: [i64; 2] = [0; 2];
    a[next()] += __rmc_ix0;
    println!("{}", a[1]);
}
