// The `matches!` macro, including the optional leading `|` Rust allows.
fn classify(x: i64) -> i64 {
    if matches!(x, 0) { 0 }
    else if matches!(x, 1..=9) { 1 }
    else if matches!(x, -9..=-1) { -1 }
    else { 2 }
}

fn main() {
    println!("{}", matches!(5, 1 | 3 | 5));
    println!("{}", matches!(4, 1 | 3 | 5));
    println!("{}", matches!(5, | 1 | 3 | 5));
    println!("{}", matches!(42, _));
    println!("{}", matches!(true, true));
    println!("{}", matches!(false, true));
    println!("{}", matches!("hi", "hi"));
    println!("{}", matches!("hi", "no"));

    let o: Option<i64> = Some(3);
    let n: Option<i64> = None;
    println!("{}", matches!(o, Some(_)));
    println!("{}", matches!(n, Some(_)));
    println!("{}", matches!(o, Some(v) if v > 2));
    println!("{}", matches!(o, Some(v) if v > 5));

    println!("{}", classify(0));
    println!("{}", classify(7));
    println!("{}", classify(-7));
    println!("{}", classify(100));
}
