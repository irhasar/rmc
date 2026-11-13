// A `mut self` receiver: taken by value like `self`, but the binding is
// mutable, so the body may assign it. rmc used to reject the method outright
// (it did not recognise `mut self` as a receiver) and then rejected the
// assignment as a write to an immutable argument.

struct Counter {
    n: i64,
}

impl Counter {
    fn new(n: i64) -> Counter {
        Counter { n }
    }

    fn bump(mut self, k: i64) -> Counter {
        self.n += k;
        self
    }

    // `&mut self` still works alongside `mut self`.
    fn bump_in_place(&mut self, k: i64) {
        self.n += k;
    }
}

fn main() {
    let c = Counter::new(1);
    let d = c.bump(4);
    println!("{}", d.n);

    let mut e = Counter::new(10);
    e.bump_in_place(5);
    println!("{}", e.n);

    // Chained: each `bump` takes the previous result by value.
    let f = Counter::new(0).bump(1).bump(2).bump(3);
    println!("{}", f.n);
}
