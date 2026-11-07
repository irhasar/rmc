struct P { x: i64, y: i64 }

impl P {
    fn sum(&self) -> i64 { self.x + self.y }
    fn bump(&mut self) -> i64 { self.x += 1; self.x }
}

fn side(v: i64) -> i64 { println!("{}", v); v * 2 }

fn double(v: i64) -> i64 { v + v }

fn wrap(p: &P) -> i64 { p.sum() + p.sum() }

fn recur(n: i64) -> i64 { if n < 2 { return n; } recur(n - 1) + recur(n - 2) }

fn main() {
    let mut p: P = P { x: 3, y: 4 };
    println!("{}", p.sum());
    println!("{}", wrap(&p));
    println!("{}", double(side(5)));
    println!("{}", double(side(1) + side(2)));
    println!("{}", p.bump());
    println!("{}", p.sum());
    println!("{}", recur(10));
}
