fn main() {
    let args: Vec<i64> = std::env::args().skip(1).map(|s| s.parse::<i64>().unwrap()).collect();
    let x = args[0];
    let d = args[1];
    println!("{}", x / -1);
    println!("{}", x % -1);
    println!("{}", x / d);
    println!("{}", x % d);
}
