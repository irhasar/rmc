use std::env;

struct Acc {
    sum: i64,
    cnt: i64,
}

fn main() {
    let n: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000000);
    let n = std::hint::black_box(n);
    let mut a = Acc { sum: 0, cnt: 0 };
    for i in 0..n {
        a.sum += i;
        if a.sum > 1000000000 {
            a.sum -= 1000000000;
            a.cnt += 1;
        }
    }
    for i in 0..=1000 {
        for j in 0..=10 {
            a.sum += i ^ j;
        }
    }
    println!("{}", a.sum);
    println!("{}", a.cnt);
}
