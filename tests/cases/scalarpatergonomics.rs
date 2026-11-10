// Scalar patterns (literal, range, bool) through a reference scrutinee: match
// ergonomics deref a `&T` to the `T` underneath, so `match &x { 5 => .. }`
// reads the `i64` rather than comparing the pointer. Only a pattern auto-derefs
// — a bare `r < 6` still needs `*r` — so the same reference used in a guard
// needs the explicit `*`. The parser cannot insert the deref (it does not know
// the type), so it marks the scrutinee and sema rewrites it.
fn lit(r: &i64) -> i64 {
    match r {
        5 => 1,
        6 => 2,
        _ => 3,
    }
}

fn range(r: &i64) -> i64 {
    match r {
        1..=10 => 1,
        11..20 => 2,
        _ => 3,
    }
}

fn or_pat(r: &i64) -> i64 {
    match r {
        1 | 5 | 9 => 1,
        _ => 2,
    }
}

fn boolean(r: &bool) -> i64 {
    match r {
        true => 1,
        false => 0,
    }
}

fn via_if_let(r: &i64) -> i64 {
    if let 5 = r {
        return 10;
    }
    20
}

fn guard_needs_deref(r: &i64) -> i64 {
    // The pattern auto-derefs, but the guard is a plain comparison and must
    // write `*r`, exactly as rustc requires.
    match r {
        n if *n > 4 => 1,
        _ => 0,
    }
}

fn main() {
    let x: i64 = 5;
    let y: i64 = 6;
    let z: i64 = 100;
    println!("{}", lit(&x));
    println!("{}", lit(&y));
    println!("{}", lit(&z));
    println!("{}", range(&x));
    println!("{}", range(&z));
    let eleven: i64 = 11;
    let nineteen: i64 = 19;
    println!("{}", range(&eleven));
    println!("{}", range(&nineteen));
    println!("{}", or_pat(&x));
    println!("{}", or_pat(&z));
    let t: bool = true;
    let f: bool = false;
    println!("{}", boolean(&t));
    println!("{}", boolean(&f));
    println!("{}", via_if_let(&x));
    println!("{}", via_if_let(&z));
    println!("{}", guard_needs_deref(&x));
    println!("{}", guard_needs_deref(&z));
}
