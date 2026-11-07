struct P { x: i64, y: i64 }

fn main() {
    let x: i64 = 3;
    let y: i64 = 4;

    let p: P = P { x, y };
    println!("{}", p.x);
    println!("{}", p.y);

    let q: P = P { y: 10, x };
    println!("{}", q.x);
    println!("{}", q.y);

    let r: P = mk(x, y);
    println!("{}", r.x + r.y);
}

fn mk(x: i64, y: i64) -> P {
    P { x, y }
}
