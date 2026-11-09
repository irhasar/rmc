// Immutable-binding write rules: a single write on each path is accepted even
// when the write sits in a method receiver or a branch that returns, and a
// `loop` that runs at most once keeps the straight-line rules.
#[derive(Clone, Copy)]
struct P {
    v: i64,
}

impl P {
    fn get(self) -> i64 {
        self.v
    }
}

// The receiver block writes `x` once; the method-call lowering must not count
// the receiver's type check twice.
fn via_receiver() -> i64 {
    let x: i64;
    let p: P = P { v: 40 };
    println!("{}", ({ x = 1; p }).get() + x);
    0
}

// One write on either route: the `if` arm that returns must not carry its write
// into the fall-through path.
fn after_return(c: bool) -> i64 {
    let x: i64;
    if c {
        x = 10;
        return x;
    }
    x = 20;
    x
}

// A `loop` whose body always reaches `break` runs at most once, so the write to
// `x` is a straight-line write and is accepted.
fn one_iter() -> i64 {
    let x: i64;
    loop {
        x = 7;
        break;
    }
    x
}

fn main() {
    via_receiver();
    println!("{}", after_return(true));
    println!("{}", after_return(false));
    println!("{}", one_iter());
}
