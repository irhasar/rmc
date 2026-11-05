// The Rust twin of `fordestr.rms`. A plain Rust subset.
#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct Q { p: P, t: i64 }

fn mk(n: i64) -> i64 {
    println!("{}", n);
    n
}

fn main() {
    let arr: [P; 3] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];
    let mut acc: i64 = 0;

    for P { x, y } in arr { acc += x * 10 + y; }

    for P { x, .. } in arr { acc += x; }
    for P { y: a, x: _ } in arr { acc += a; }

    for P { mut x, y } in arr { x = x + 100; acc += x + y; }

    let qs: [Q; 2] = [Q { p: P { x: 7, y: 8 }, t: 1 }, Q { p: P { x: 9, y: 10 }, t: 2 }];
    for Q { p, t } in qs {
        let P { x, y } = p;
        acc += x + y + t;
    }

    for P { x, .. } in [P { x: mk(11), y: mk(12) }, P { x: mk(13), y: mk(14) }] {
        acc += x;
    }

    for _ in [P { x: mk(15), y: 0 }, P { x: mk(16), y: 0 }] { acc += 1; }

    println!("{}", acc);
}
