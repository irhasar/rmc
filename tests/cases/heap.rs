fn main() {
    let mut ok = 0i64;
    for _ in 0..1000 {
        let v: Vec<i64> = Vec::with_capacity(6);
        if v.capacity() >= 6 {
            ok += 1;
        }
    }
    println!("{}", ok);

    // `Vec::new()` is the empty constructor: no allocation, a zero length.
    let e: Vec<i64> = Vec::new();
    println!("{} {}", e.len(), e.capacity());
}
