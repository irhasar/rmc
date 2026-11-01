fn f(a: i64, b: i64) -> i64 {
    a * b - a + b
}
fn g(x: i64) -> i64 {
    if x <= 0 {
        return 0;
    }
    x + g(x - 1)
}
fn main() {
    let mut a: [i64; 6] = [0; 6];
    let mut i: i64 = 0;
    while i < 6 {
        a[i as usize] = i + 1;
        i += 1;
    }
    println!("{}", f(a[0], a[1]) + f(a[2], a[3]) - f(a[4], a[5]));
    println!("{}", g(10));
    println!("{}", (a[0] + a[1]) * (a[2] + a[3]) - (a[4] * a[5]));
    let x: i64 = 5;
    {
        let x: i64 = x + 100;
        println!("{}", x);
        {
            let x: i64 = x * 2;
            println!("{}", x);
        }
        println!("{}", x);
    }
    println!("{}", x);
    let mut c: i64 = 0;
    c = c + f(1, 2) + f(3, 4) + f(5, 6);
    println!("{}", c);
    println!("{}", if a[1] < a[2] { g(3) } else { g(4) });
    println!("{}", f(f(1, 2), f(3, 4)));
}
