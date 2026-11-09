// The Rust twin of `refidxorder.rms`: the index block writes `y`, but the
// assigned value is read first, so `r[0]` is 1 and `y` ends at 2.
fn main() {
    let mut y: i64 = 1;
    let mut a: [i64; 1] = [0; 1];
    let r: &mut [i64; 1] = &mut a;
    r[{ y = 2; 0 }] = y;
    println!("{}", r[0]);
    println!("{}", y);
}
