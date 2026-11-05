// Rust twin of forarr.rms.
fn mk() -> [i64; 3] {
    println!("{}", -1);
    [5, 6, 7]
}

fn main() {
    let a: [i64; 4] = [1, 2, 3, 4];
    let mut s: i64 = 0;
    for x in a {
        s += x;
    }
    println!("{}", s);

    let mut t: i64 = 0;
    for x in [10, 20, 30] {
        t += x;
    }
    println!("{}", t);

    let mut u: i64 = 0;
    for x in mk() {
        u += x;
    }
    println!("{}", u);

    let mut c: i64 = 0;
    for x in a {
        if x == 2 { continue; }
        if x == 4 { break; }
        c += x;
    }
    println!("{}", c);

    let mut n: i64 = 0;
    for _ in a {
        n += 1;
    }
    println!("{}", n);

    let b: [i64; 2] = [2, 3];
    let mut p: i64 = 0;
    for x in a {
        for y in b {
            p += x * y;
        }
    }
    println!("{}", p);
    println!("{}", a[1]);

    let one: [i64; 1] = [42];
    let mut q: i64 = 0;
    for x in one {
        q += x;
    }
    println!("{}", q);
}
