// A running difference with an immediate term. `acc -= 1` folds into a single
// `subq $1, <acc>`, which the in-process encoder used to reject (it handled
// `addq` but not `subq`). The `as`+`ld` path accepted it, so only the default
// in-process link failed.

struct Counter {
    n: i64,
}

impl Counter {
    fn dec(&mut self) {
        self.n -= 1;
    }
    fn sub(&mut self, k: i64) {
        self.n -= k;
    }
}

fn main() {
    let mut c = Counter { n: 10 };
    c.dec();
    c.dec();
    println!("{}", c.n);
    c.sub(3);
    println!("{}", c.n);
}
