// Formatting a reference must print the referent, not the address. `println!`
// on `&T` for `T` scalar goes through a deref trampoline (rustc calls the base
// formatter with a pointer to the referent; rmc's runtime is handed the slot).
// A `&str` referent is a slice header, so the runtime's own deref already
// reaches the record and the trampoline must not double-deref it.
fn main() {
    let n = 42i64;
    let r = &n;
    println!("{}", r);
    println!("{:?}", r);
    println!("{:x}", r);
    let b = true;
    println!("{}", &b);
    let s = "hello";
    let rs = &s;
    println!("{}", rs);
    println!("{:?}", rs);
    let q = &&r;
    println!("{}", q);
    let arr = [1, 2, 3];
    for x in &arr {
        println!("{}", x);
    }
    // Two functions formatting a `&i64` must share one deref trampoline, not
    // emit a duplicate label.
    println!("{}", twice(&n) + also(&n));
}

fn twice(x: &i64) -> i64 {
    println!("{}", x);
    *x
}

fn also(x: &i64) -> i64 {
    println!("{}", x);
    *x + 1
}
