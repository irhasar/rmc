#[derive(Copy, Clone)]
struct I { u: i64, v: i64 }
#[derive(Copy, Clone)]
struct S { a: [i64; 2], b: i64 }
#[derive(Copy, Clone)]
struct T { p: [I; 2], q: i64 }

fn main() {
    let s = S { a: [3, 4], b: 5 };

    let S { a: [x, y], b } = s;
    println!("{}", x * 100 + y * 10 + b);

    let S { a: [_, y2], .. } = s;
    println!("{}", y2);
    let S { a: [p, ..], b: _ } = s;
    println!("{}", p);

    let t = T { p: [I { u: 1, v: 2 }, I { u: 3, v: 4 }], q: 5 };
    let T { p: [I { u, v }, I { u: u2, .. }], q } = t;
    println!("{}", u * 1000 + v * 100 + u2 * 10 + q);

    let r = &s;
    match r {
        S { a: [m, n], b } => println!("{}", m * 100 + n * 10 + b),
    }

    let arr = [s, S { a: [7, 8], b: 9 }];
    let mut acc = 0;
    for S { a: [m, n], .. } in arr {
        acc = acc + m * 10 + n;
    }
    println!("{}", acc);

    if let S { a: [m, ..], .. } = s {
        println!("{}", m);
    }

    let S { a: [mut mm, nn], .. } = s;
    mm = mm + 100;
    println!("{}", mm + nn);
}
