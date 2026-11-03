struct P { a: i64, b: i64 }

impl P {
    fn sum(&self) -> i64 { return self.a + self.b; }
    fn prod(&self) -> i64 { return self.a * self.b; }
}

fn main() {
    let n: i64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let n = std::hint::black_box(n);
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let p: P = P { a: i + 1, b: i + 2 };
        acc = acc + p.sum() + p.prod();
        i = i + 1;
    }
    println!("{}", acc);
    let q: P = P { a: 3, b: 4 };
    let mut j: i64 = 0;
    while j < n {
        println!("{}", q.sum());
        j = j + 1;
    }
    let k: i64 = 7;
    let mut m: i64 = 0;
    let mut t: i64 = 0;
    while t < n {
        m = m + (k * k + k);
        t = t + 1;
    }
    println!("{}", m);
}
