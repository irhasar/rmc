// Rust twin of tuple.rms.
fn swap(p: (i64, i64)) -> (i64, i64) {
    (p.1, p.0)
}

fn add(p: (i64, i64)) -> i64 {
    p.0 + p.1
}

fn sum3(t: (i64, i64, i64)) -> i64 {
    t.0 + t.1 + t.2
}

fn diff((a, b): (i64, i64)) -> i64 {
    a - b
}

fn main() {
    let t = (1, 2);
    println!("{}", t.0);
    println!("{}", t.1);

    let s = swap(t);
    println!("{}", s.0);
    println!("{}", s.1);
    println!("{}", add(s));

    let (a, mut b) = s;
    b = b + 10;
    println!("{}", a);
    println!("{}", b);

    let u: (i64, bool) = (7, true);
    if u.1 {
        println!("{}", u.0);
    }

    let nested = ((1, 2), 3);
    println!("{}", nested.0 .0);
    println!("{}", nested.0 .1);
    println!("{}", nested.1);

    println!("{}", sum3((10, 20, 30)));

    println!("{}", diff((9, 4)));

    let pairs: [(i64, i64); 2] = [(1, 2), (3, 4)];
    let mut total = 0;
    for (x, y) in pairs {
        total = total + x * 10 + y;
    }
    println!("{}", total);

    match (7, 8) {
        (x, y) => {
            println!("{}", x * 100 + y);
        }
    }

    let (only, _) = (99, 0);
    println!("{}", only);

    println!("{}", add((4, 5)));

    std::process::exit(0);
}
