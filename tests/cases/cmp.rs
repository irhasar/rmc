// rmc's print_bool writes 1 or 0, so the twin mirrors that with `pb`.
fn pb(b: bool) {
    println!("{}", b as i64);
}
fn main() {
    let x: i64 = 10;
    pb(5 < x);
    pb(x < 5);
    pb(5 > x);
    pb(x > 5);
    pb(10 <= x);
    pb(11 <= x);
    pb(x <= 10);
    pb(x <= 9);
    pb(10 >= x);
    pb(9 >= x);
    pb(x >= 10);
    pb(x >= 11);
    pb(10 == x);
    pb(11 == x);
    pb(10 != x);
    pb(11 != x);
    let y: i64 = 3;
    pb(2 < y);
    pb(2 > y);
    pb(3 < y);
    pb(3 > y);
    pb(0 <= y);
    pb(0 >= y);
    pb(y < y);
    pb(y <= y);
    pb(y >= y);
    pb(y == y);
    pb(y != y);
}
