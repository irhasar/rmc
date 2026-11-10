// A `match` whose every arm diverges has type `!`, so it is accepted as an
// `if`/`while` condition exactly as the equivalent `if`/`else` chain is: the
// condition never yields a value, and the branch it guards is unreachable.
// A non-exhaustive or partially-diverging chain is still rejected.
fn pick(b: bool) -> i64 {
    if match b { true => { return 1; }, false => { return 2; } } {}
    return 0;
}

fn loop_pick(b: bool) -> i64 {
    while match b { true => { return 3; }, false => { return 4; } } {}
    return 0;
}

fn enum_pick(e: E) -> i64 {
    if match e { E::A => { return 5; }, E::B => { return 6; } } {}
    return 0;
}

enum E { A, B }

fn main() {
    println!("{}", pick(0 < 1));
    println!("{}", pick(1 < 0));
    println!("{}", loop_pick(0 < 1));
    println!("{}", loop_pick(1 < 0));
    println!("{}", enum_pick(E::A));
    println!("{}", enum_pick(E::B));
}
