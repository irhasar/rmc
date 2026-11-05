// Rust twin of nestpat.rms.
#[derive(Copy, Clone)]
struct I { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct S { a: I, b: i64 }

fn sum(i: &I) -> i64 {
    i.x * 10 + i.y
}

fn main() {
    let s = S { a: I { x: 3, y: 4 }, b: 5 };

    let S { a: I { x, y }, b } = s;
    println!("{}", x * 100 + y * 10 + b);

    let S { a: I { x: p, y: _ }, .. } = s;
    println!("{}", p);

    let S { a: I { .. }, b: _ } = s;
    println!("{}", 0);

    let r = &s;
    let mut acc: i64 = 0;
    match r {
        S { a: I { x, y }, b } => { acc = acc + x * 100 + y * 10 + b; }
    }
    println!("{}", acc);

    let deep = S { a: I { x: 7, y: 8 }, b: 9 };
    let S { a: I { x: dx, y: dy }, b: db } = deep;
    println!("{}", dx * 100 + dy * 10 + db);

    let arr = [s, deep];
    let mut total: i64 = 0;
    for S { a: I { x, y }, b } in arr {
        total = total + x + y + b;
    }
    println!("{}", total);

    if let S { a: I { x, .. }, .. } = s {
        println!("{}", x);
    }

    println!("{}", sum(&s.a));
}
