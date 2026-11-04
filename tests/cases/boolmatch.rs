// `match` on a `bool`: the `true`/`false` arms together cover every value, so
// the desugared chain is total and may stand for a value. The pattern desugars
// to a `BoolTest`, not a plain `==`, which is what lets sema see the chain as
// exhaustive; a hand-written `if b == true { .. } else if b == false { .. }`
// would not be. A call in the scrutinee runs exactly once.
fn to_num(b: bool) -> i64 {
    match b {
        true => 1,
        false => 0,
    }
}

fn both_ways(b: bool) -> i64 {
    let x: i64 = match b { true => 10, false => 20 };
    x
}

fn side(k: i64) -> bool {
    println!("{}", k);
    k > 0
}

fn main() {
    println!("{}", to_num(1 < 2));
    println!("{}", to_num(2 < 1));
    println!("{}", both_ways(0 < 1));
    println!("{}", both_ways(1 < 0));

    // A non-trivial scrutinee is evaluated once, however many arms test it.
    println!("{}", to_num(side(7)));
    println!("{}", to_num(side(0 - 3)));

    // A bool match as a statement, with block arms.
    let c: bool = 3 < 4;
    match c {
        true => { println!("11"); }
        false => { println!("22"); }
    }
}
