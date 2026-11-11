// A plain Rust subset.
#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct Inner { a: i64, b: i64 }
#[derive(Copy, Clone)]
struct Q { i: Inner, y: i64 }
#[derive(Copy, Clone)]
struct Arr { v: [i64; 3], n: i64 }

fn side(k: i64) -> i64 {
    println!("{}", k);
    k * 2
}

fn mkp(k: i64) -> P {
    println!("{}", k);
    P { x: k, y: k + 1 }
}

fn first(p: P) -> i64 {
    let P { x, .. } = p;
    x
}

fn main() {
    let p: P = P { x: 3, y: 4 };

    let P { x, y: z } = p;
    println!("{}", x * 10 + z);

    let P { x: a, y: _ } = p;
    println!("{}", a);
    let P { y, .. } = p;
    println!("{}", y);
    let P { .. } = p;

    let P { x, y }: P = p;
    println!("{}", x + y);

    let P { mut x, y: mut w } = p;
    x = 100;
    w = 200;
    println!("{}", x + w);

    let P { x, y } = P { x: 7, y: 8 };
    println!("{}", x * 10 + y);
    if true {
        let P { x, .. } = p;
        println!("{}", x + 1000);
    }
    println!("{}", x);

    let P { x, y } = mkp(50);
    println!("{}", x + y);

    let P { x, y } = P { x: side(1), y: side(2) };
    println!("{}", x + y);

    let r: &P = &p;
    let P { x, y } = r;
    println!("{}", x * 100 + y);

    let q: Q = Q { i: Inner { a: 5, b: 6 }, y: 7 };
    let Q { i, y } = q;
    let Inner { a, b } = i;
    println!("{}", a * 1000 + b * 100 + y);

    let ar: Arr = Arr { v: [4, 5, 6], n: 9 };
    let Arr { v, n } = ar;
    println!("{}", v[0] + v[1] + v[2] + n);

    println!("{}", first(P { x: 11, y: 22 }));

    let P { .. } = P { x: side(7), y: side(8) };
}
