// Arguments run left to right, whatever their kind. A scalar or an aggregate
// argument with a side effect must fire in source order; `f(sc(1), mk(2))`
// prints 1 then 2. Aggregates are still passed by value (copied into a slot).
#[derive(Copy, Clone)]
struct S { v: i64 }

fn mk(t: i64) -> S {
    println!("{}", t);
    S { v: t }
}

fn sc(t: i64) -> i64 {
    println!("{}", t);
    t * 10
}

fn mix(a: i64, s: S, b: i64) -> i64 {
    a + s.v + b
}

fn mix2(s: S, a: i64) -> i64 {
    s.v * 100 + a
}

fn main() {
    println!("{}", mix(sc(1), mk(2), sc(3)));
    println!("{}", mix2(mk(4), sc(5)));
}
