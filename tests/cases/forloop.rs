fn main() {
    let mut s: i64 = 0;
    for i in 0..10 {
        if i == 3 {
            continue;
        }
        if i == 8 {
            break;
        }
        s += i;
    }
    println!("{}", s);
    let mut t: i64 = 0;
    for a in 0..4 {
        for b in 0..4 {
            t += a * b;
        }
    }
    println!("{}", t);
    let n: i64 = 6;
    let mut u: i64 = 0;
    for k in 0..n {
        u += k;
    }
    println!("{}", u);
    let mut v: i64 = 0;
    for i in 0..=10 {
        v += i;
    }
    println!("{}", v);
    let mut w: i64 = 0;
    for i in 5..=3 {
        w += i;
    }
    println!("{}", w);
    let mut z: i64 = 0;
    for i in 7..=7 {
        z += i;
    }
    println!("{}", z);
    let m: i64 = 4;
    let mut q: i64 = 0;
    for i in 1..=m {
        q += i;
    }
    println!("{}", q);
}
