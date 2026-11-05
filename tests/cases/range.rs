// Rust twin of range.rms.
fn bucket(x: i64) -> i64 {
    match x {
        0 => 0,
        1..=5 => 1,
        6..10 => 2,
        -5..0 => 3,
        _ => 4,
    }
}

fn main() {
    println!("{}", bucket(0));
    println!("{}", bucket(3));
    println!("{}", bucket(5));
    println!("{}", bucket(6));
    println!("{}", bucket(9));
    println!("{}", bucket(10));
    println!("{}", bucket(-3));
    println!("{}", bucket(-5));
    println!("{}", bucket(100));

    // A range arm and an or-pattern arm in the same match.
    let x: i64 = 100;
    println!("{}", match x { 1..=5 => 1, 90 | 100 => 2, _ => 0 });

    // Range in `if let`.
    let y: i64 = 4;
    if let 1..=5 = y {
        println!("{}", 1);
    } else {
        println!("{}", 0);
    }

    // A range over the whole i64 domain needs no `_` arm to be usable as a
    // value only when a `_` is present; here it is a statement.
    let z: i64 = 42;
    match z {
        -9223372036854775808..=9223372036854775807 => {
            println!("{}", 7);
        }
        _ => {
            println!("{}", 0);
        }
    }
}
