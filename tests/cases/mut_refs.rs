// The Rust twin of `mut_refs.rms`.
#[derive(Copy, Clone)]
struct C { n: i64 }

impl C {
    fn bump(&mut self) -> i64 { self.n += 1; self.n }
    fn read(&self) -> i64 { self.n }
}

fn inc(p: &mut i64) -> i64 { *p += 1; *p }

fn peek(p: &i64) -> i64 { *p }

fn main() {
    let mut c = C { n: 10 };
    let cm: &mut C = &mut c;
    println!("{}", cm.bump());
    println!("{}", cm.read());
    println!("{}", cm.bump());

    let mut x: i64 = 1;
    println!("{}", inc(&mut x));
    println!("{}", inc(&mut x));
    println!("{}", x);
    println!("{}", peek(&mut x));

    println!("{}", c.bump());
    println!("{}", c.read());
}
