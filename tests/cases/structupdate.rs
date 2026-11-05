// The Rust twin of `structupdate.rms`. A plain Rust subset.
#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct Inner { a: i64 }
#[derive(Copy, Clone)]
struct Q { i: Inner, y: i64 }

fn mk() -> P { P { x: 7, y: 8 } }

fn mkq(g: Inner) -> Q { Q { i: g, y: 2 } }

fn sum(p: P) -> i64 { p.x + p.y }

fn main() {
    let a: P = P { x: 1, y: 2 };

    let b: P = P { x: 5, ..a };
    println!("{}", b.x);
    println!("{}", b.y);

    let c: P = P { y: 20, ..a };
    println!("{}", c.x);
    println!("{}", c.y);

    let d: P = P { x: 1, ..mk() };
    println!("{}", d.x);
    println!("{}", d.y);

    let mut e: P = P { x: 100, ..a };
    e.y = 99;
    println!("{}", a.y);
    println!("{}", e.y);

    let f: P = P { ..a };
    let g: P = P { x: 3, ..f };
    println!("{}", g.x + g.y);

    let inner: Inner = Inner { a: 5 };
    let q: Q = mkq(inner);
    let r: Q = Q { y: 9, ..q };
    println!("{}", r.i.a);
    println!("{}", r.y);

    println!("{}", sum(P { x: 30, ..a }));
    println!("{}", sum(P { x: 40, ..P { y: 50, ..a } }));
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < 4 {
        let p: P = P { x: i, ..a };
        acc = acc + p.x + p.y;
        i += 1;
    }
    println!("{}", acc);
}
