// rmc's print_bool writes 1 or 0, so the twin mirrors that with `pb`.
fn pb(b: bool) {
    println!("{}", b as i64);
}
fn side(n: i64) -> bool {
    println!("{}", n);
    true
}
fn main() {
    pb(true && true);
    pb(true && false);
    pb(false && true);
    pb(false || false);
    pb(true || false);
    pb(!true);
    pb(!false);
    pb(!(3 > 5));
    pb((1 < 2) && (3 < 4));
    pb((1 < 2) || (3 > 4));
    // Short-circuit: side(1) must not run, side(2) must.
    pb(false && side(1));
    pb(true || side(2));
    pb(true && side(3));
    let mut b: bool = 5 < 6;
    println!("{}", b as i64);
    b = false;
    println!("{}", b as i64);
    let n: i64 = 1;
    pb(n != 0);
    let z: i64 = 0;
    pb(z != 0);
}
