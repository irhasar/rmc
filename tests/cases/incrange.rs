// Differential twin of `incrange.rms`: an inclusive range ending at `i64::MAX`
// must terminate, and `MAX..=MAX` runs exactly once.
fn main() {
    let mut c = 0i64;
    for _ in 9223372036854775805i64..=9223372036854775807 {
        c += 1;
    }
    println!("{}", c);
    let mut d = 0i64;
    for i in 0..=3 {
        d += i;
    }
    println!("{}", d);
    let mut e = 0i64;
    for _ in 9223372036854775807i64..=9223372036854775807 {
        e += 1;
    }
    println!("{}", e);
}
