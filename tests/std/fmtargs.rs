fn main() {
    let a: i64 = 42;
    let b: i64 = -7;
    let c: u32 = 4000000000;
    let t: bool = true;
    let f: bool = false;
    println!("a={}", a);
    println!("b={}", b);
    println!("a={} b={}", a, b);
    println!("{a} {b}");
    println!("c={} t={} f={}", c, t, f);
    println!("{} {} {}", 1, 2, 3);
    println!("mixed {} then {}", a, "text");
    print!("no trailing newline: {}", a);
    println!("!");
}
