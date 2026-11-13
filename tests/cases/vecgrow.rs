// A growable `Vec<T>`: `Vec::new()`/`Vec::with_capacity(k)` then `push`. The
// length and capacity are runtime values, so both are read back after pushes
// that force the store to grow; the doubling policy matches `std`, so the
// capacity sequence is pinned exactly. The elements are then read back by index
// and by reference, and a `&mut` iteration writes through to the store.
fn main() {
    // Grow from empty: capacity follows std's doubling from the minimum
    // non-zero capacity (4 for an 8-byte element).
    let mut v: Vec<i64> = Vec::new();
    for i in 0..20 {
        v.push(i * 2);
        print!("{}:{} ", v.len(), v.capacity());
    }
    println!("");
    for i in 0..20 {
        print!("{} ", v[i]);
    }
    println!("");

    // Grow from a known capacity.
    let mut w: Vec<i64> = Vec::with_capacity(2);
    w.push(100);
    w.push(200);
    w.push(300);
    w.push(400);
    println!("{} {}", w.len(), w.capacity());

    // A byte element has a one-byte stride, so the minimum capacity is 8.
    let mut b: Vec<u8> = Vec::new();
    for i in 0..20 {
        b.push((i * 3) as u8);
    }
    println!("{} {}", b.len(), b.capacity());
    for i in 0..20 {
        print!("{} ", b[i]);
    }
    println!("");

    // A list literal is exactly sized: capacity equals length.
    let l: Vec<i64> = vec![11, 22, 33];
    println!("{} {}", l.len(), l.capacity());

    // Read by reference and write through a `&mut` iteration.
    let mut m: Vec<i64> = vec![1, 2, 3, 4];
    for x in &m {
        print!("{} ", *x);
    }
    println!("");
    for x in m.iter_mut() {
        *x *= 10;
    }
    for x in &m {
        print!("{} ", *x);
    }
    println!("");

    // Push onto a literal-sized vector forces a grow from capacity == length.
    let mut g: Vec<i64> = vec![7, 8, 9];
    g.push(10);
    println!("{} {}", g.len(), g.capacity());
    for i in 0..g.len() {
        print!("{} ", g[i]);
    }
    println!("");

    // `is_empty()` reads the runtime length; a by-value `for` consumes the
    // vector and reads its elements.
    let e: Vec<i64> = Vec::new();
    println!("{} {}", e.is_empty(), g.is_empty());
    let h: Vec<i64> = vec![1, 2, 3];
    for x in h {
        print!("{} ", x);
    }
    println!("");

    // Growth driven by a `while` loop and a large grow; the write-back keeps the
    // binding pointing at the current store after each reallocation.
    let mut p: Vec<i64> = Vec::new();
    let mut i = 0;
    while i < 5 {
        p.push(i);
        i += 1;
    }
    println!("{} {}", p.len(), p.capacity());
    let mut q: Vec<i64> = Vec::new();
    for k in 0..100 {
        q.push(k);
    }
    let mut s = 0;
    for x in &q {
        s += *x;
    }
    println!("{} {}", q.len(), s);
}
