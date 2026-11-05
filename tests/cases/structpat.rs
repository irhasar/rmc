// The Rust twin of `structpat.rms`. A plain Rust subset.
#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct Inner { a: i64, b: i64 }
#[derive(Copy, Clone)]
struct Q { i: Inner, y: i64 }

fn early(p: Late) -> i64 {
    match p {
        Late { n, .. } => n * 2,
    }
}

#[derive(Copy, Clone)]
struct Late { n: i64, m: i64 }

fn mkp() -> P { P { x: 8, y: 9 } }

fn classify(p: P) -> i64 {
    match p {
        P { x, y } => x * 100 + y,
    }
}

fn sum_ref(p: &P) -> i64 {
    match p {
        P { x, y } => x + y,
    }
}

fn main() {
    let p: P = P { x: 3, y: 4 };

    match p {
        P { x, y } => {
            println!("{}", x);
            println!("{}", y);
        }
    }
    println!("{}", classify(p));

    match p {
        P { x: a, y: _ } => println!("{}", a),
    }

    match p {
        P { y, .. } => println!("{}", y * 10),
    }

    if let P { x, y } = p {
        println!("{}", x + y);
    } else {
        println!("{}", -1);
    }

    match p {
        P { x, y } if x > y => println!("{}", 1),
        _ => println!("{}", 0),
    }

    let q: Q = Q { i: Inner { a: 5, b: 6 }, y: 7 };
    match q {
        Q { i, y } => {
            match i {
                Inner { a, b } => println!("{}", a * 1000 + b * 100 + y),
            }
        }
    }

    println!("{}", early(Late { n: 21, m: 0 }));

    println!("{}", sum_ref(&p));

    match mkp() {
        P { x, y } => println!("{}", x - y),
    }

    match p {
        P { .. } => println!("{}", 0),
    }
}
