// Rust twin of atpat.rms.
struct P {
    x: i64,
    y: i64,
    z: i64,
}

fn takes(n @ _: i64, mut m @ _: i64) -> i64 {
    m += n;
    m
}

fn pair(t @ (a, b): (i64, i64)) -> i64 {
    t.0 + t.1 + a + b
}

fn main() {
    // A plain alias and a chain of aliases.
    let x = 10;
    let n @ _ = x;
    let p @ q @ _ = x;
    println!("{}", n + p + q);

    // A tuple sub-pattern: the whole tuple and each element bound.
    let t @ (a, c) = (3, 4);
    println!("{}", t.0 + t.1 + a + c);

    // A struct field sub-pattern, with a `mut` alias that is written.
    let s = P { x: 1, y: 2, z: 3 };
    let P { x: r @ _, y: mut w @ _, .. } = s;
    w += 1;
    println!("{}", r + w);

    // `for` over a range: a plain alias and a second binding.
    let mut acc: i64 = 0;
    for i @ _ in 0..4 {
        acc += i;
    }
    println!("{}", acc);
    for i @ j in 0..3 {
        acc += i + j;
    }
    println!("{}", acc);

    // `for` over an array.
    let arr = [5, 6, 7];
    for e @ _ in arr {
        acc += e;
    }
    println!("{}", acc);

    // A `mut` alias inside an array and a tuple element.
    let two = [5, 6];
    let [mut d @ _, h] = two;
    d += 1;
    println!("{}", d + h);
    let (mut u @ _, v) = (1, 2);
    u += 1;
    println!("{}", u + v);

    // A refutable `@` in a `match` arm and an `if let`.
    match x {
        k @ 1..=100 => {
            println!("{}", k);
        }
        _ => {}
    }
    if let k @ 1..=100 = x {
        println!("{}", k + 1);
    }

    // Parameters, including a `mut` alias and a tuple sub-pattern.
    println!("{}", takes(x, x + 1));
    println!("{}", pair((x, x + 2)));
    std::process::exit(0);
}
