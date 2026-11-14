fn main() {
    let a: i64 = 17;
    let b: i64 = 5;
    println!("{} {} {} {}", a + b, a - b, a * b, a / b);
    let mut s = 0i64;
    for i in 0..10 {
        s += i * i;
    }
    println!("sum={}", s);
}
