// The Rust twin of `paramaddr.rms`.
fn readref(x: i64) -> i64 { let r = &x; *r }

fn writeback(mut x: i64) -> i64 { let r = &mut x; *r = 5; x }

fn bumpmut(mut x: i64) -> i64 { let r = &mut x; *r += 10; *r }

fn main() {
    println!("{}", readref(3));
    let mut y: i64 = 3;
    println!("{}", writeback(y));
    println!("{}", y);
    println!("{}", bumpmut(y));
    println!("{}", y);
    println!("{}", writeback(2 + 4));
}
