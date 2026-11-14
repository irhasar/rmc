// `&[T]` and `&str` indexing must account for rmc's `{len, bytes..}` record
// header: the record's first word is the length, not the first element.
fn sum(xs: &[i64]) -> i64 {
    let mut s = 0;
    let mut i = 0;
    while i < xs.len() {
        s += xs[i];
        i += 1;
    }
    s
}

fn main() {
    let a = [10, 20, 30, 40];
    println!("{}", sum(&a));
    let b = [1, 2, 3];
    println!("{}", sum(&b));
    let s = "hello";
    let bytes = s.as_bytes();
    println!("{} {} {}", bytes[0], bytes[4], bytes.len());
    let mut t = 0;
    let mut i = 0;
    while i < bytes.len() {
        t += bytes[i];
        i += 1;
    }
    println!("{}", t);
}
