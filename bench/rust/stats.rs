use std::env;

struct Acc {
    sum: i64,
    count: i64,
    max: i64,
    min: i64,
}

fn main() {
    let n: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000000);
    let n = std::hint::black_box(n);
    let mut a = Acc {
        sum: 0,
        count: 0,
        max: i64::MIN + 1,
        min: i64::MAX,
    };
    let mut i: i64 = 1;
    while i < n {
        let v: i64 = (i.wrapping_mul(2654435761)) % 1000003;
        a.sum += v;
        a.count += 1;
        if v > a.max {
            a.max = v;
        }
        if v < a.min {
            a.min = v;
        }
        i += 1;
    }
    println!("{}", a.sum);
    println!("{}", a.max);
    println!("{}", a.min);
    println!("{}", a.count);
}
