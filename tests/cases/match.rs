// `match` on integers: each arm is an integer literal (optionally negated) or
// `_`. It desugars to a chain of `if`s, so the first equal arm runs and `_`
// matches anything. It is usable as a statement or a value, and a call in the
// scrutinee runs exactly once.
fn classify(x: i64) -> i64 {
    match x {
        0 => return 100,
        1 => return 200,
        _ => return 300,
    }
}

fn sign(x: i64) -> i64 {
    let s: i64 = match x {
        -1 => 11,
        0 => 22,
        1 => 33,
        _ => 44,
    };
    s
}

fn main() {
    println!("{}", classify(0));
    println!("{}", classify(1));
    println!("{}", classify(5));

    // Value position, with a negative pattern.
    println!("{}", sign(-1));
    println!("{}", sign(0));
    println!("{}", sign(1));
    println!("{}", sign(9));

    // Statement position, arms are blocks, and no trailing comma is needed.
    let v: i64 = 2;
    match v {
        1 => { println!("1000"); }
        2 => { println!("2000"); }
        _ => { println!("3000"); }
    }

    // Nested match and a match inside a loop.
    let mut i: i64 = 0;
    let mut acc: i64 = 0;
    while i < 9 {
        acc += match i % 3 {
            0 => 100,
            1 => 10,
            _ => 1,
        };
        i += 1;
    }
    println!("{}", acc);
}
