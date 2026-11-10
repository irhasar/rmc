struct Point {
    x: i64,
    y: i64,
}

fn dist2(a: i64, b: i64) -> i64 {
    a * a + b * b
}

fn main() {
    let mut p = Point { x: 3, y: 4 };
    println!("{}", p.x);
    println!("{}", p.y);
    println!("{}", dist2(p.x, p.y));
    p.x = p.x + 100;
    p.y = -p.y;
    println!("{}", p.x);
    println!("{}", p.y);
    let q = Point { y: 7, x: -2 };
    println!("{}", q.x + q.y);
    let r = Point { x: 1, y: 1 };
    println!("{}", r.x * r.y);
}
