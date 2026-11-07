#[derive(Copy, Clone)]
struct Vec2 {
    x: i64,
    y: i64,
}

fn add(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: a.x + b.x, y: a.y + b.y }
}

fn scale(v: Vec2, k: i64) -> Vec2 {
    Vec2 { x: v.x * k, y: v.y * k }
}

fn sum3(a: [i64; 3]) -> i64 {
    a[0] + a[1] + a[2]
}

fn make3(x: i64) -> [i64; 3] {
    [x, x + 1, x + 2]
}

fn bump(mut v: Vec2) -> i64 {
    v.x += 1000;
    v.x + v.y
}

fn main() {
    let a = Vec2 { x: 1, y: 2 };
    let b = Vec2 { x: 10, y: 20 };
    let c = add(a, b);
    println!("{}", c.x);
    println!("{}", c.y);
    let d = scale(c, 3);
    println!("{}", d.x);
    println!("{}", d.y);
    let e = add(Vec2 { x: 5, y: 5 }, Vec2 { x: -1, y: -1 });
    println!("{}", e.x);
    println!("{}", e.y);
    println!("{}", bump(a));
    println!("{}", a.x);
    let arr: [i64; 3] = [4, 5, 6];
    println!("{}", sum3(arr));
    let m: [i64; 3] = make3(10);
    println!("{}", m[0]);
    println!("{}", m[1]);
    println!("{}", m[2]);
    println!("{}", sum3(make3(100)));
}
