fn main() {
    println!("{}", 2 + 3 * 4);
    println!("{}", (2 + 3) * 4);
    println!("{}", 20 - 5 - 3);
    println!("{}", 100 / 7);
    println!("{}", 100 % 7);
    println!("{}", -7 / 2);
    println!("{}", -7 % 2);
    println!("{}", 7 / -2);
    println!("{}", 0 - 5);
    println!("{}", 2 * 3 * 4 * 5);
    let mut x: i64 = 17;
    x = x + 3;
    x = x * 2;
    x = x - 5;
    x = x / 3;
    println!("{}", x);
}
