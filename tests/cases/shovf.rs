fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut y: i64 = 1;
    println!("{}", y << n);
    println!("{}", y >> n);
    y <<= n;
    println!("{}", y);
}
