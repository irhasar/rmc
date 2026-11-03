fn bump(x: i64) -> i64 {
    let mut x = x;
    x = x + 1;
    x
}

fn clamp(x: i64) -> i64 {
    let mut x = x;
    if x > 10 {
        x = 10;
    }
    x
}

fn main() {
    let a: i64 = 5;
    println!("{}", bump(a));
    println!("{}", a);

    let b: i64 = 20;
    println!("{}", clamp(b));
    println!("{}", b);
}
