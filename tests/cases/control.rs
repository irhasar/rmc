fn classify(n: i64) -> i64 {
    if n < 0 {
        -1
    } else if n == 0 {
        0
    } else {
        1
    }
}
fn main() {
    println!("{}", classify(-4));
    println!("{}", classify(0));
    println!("{}", classify(9));
    let mut i: i64 = 0;
    let mut s: i64 = 0;
    while i < 10 {
        i += 1;
        if i % 2 == 0 {
            continue;
        }
        s += i;
    }
    println!("{}", s);
    let mut k: i64 = 0;
    loop {
        k += 1;
        if k >= 7 {
            break;
        }
    }
    println!("{}", k);
    let mut total: i64 = 0;
    let mut a: i64 = 0;
    while a < 4 {
        let mut b: i64 = 0;
        while b < 4 {
            if b == 2 {
                b += 1;
                continue;
            }
            total += a * 10 + b;
            b += 1;
        }
        a += 1;
    }
    println!("{}", total);
    let v: i64 = if s > 10 { 100 } else { 200 };
    println!("{}", v);
    let mut w: i64 = 5;
    loop {
        if w == 0 {
            break;
        }
        w -= 1;
    }
    println!("{}", w);
}
