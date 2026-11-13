// A growable `String`: `String::new()`/`String::with_capacity(k)`/`String::from`
// then `push_str`/`push`. A `String` shares `str`'s `{len, bytes}` record layout,
// so `len`/`as_str`/comparison and the `str` searches all read it unchanged,
// while `capacity` reads the word before the length. The growth policy matches
// `std` (`max(2*cap, needed)`, minimum 8), so the capacity sequence is pinned.
fn main() {
    let s = String::from("abc");
    println!("{} {}", s.len(), s.capacity());

    let e = String::new();
    println!("{} {} {}", e.len(), e.capacity(), e.is_empty());

    let w = String::with_capacity(10);
    println!("{} {}", w.len(), w.capacity());

    // Grow in place, then force a reallocation; the write-back keeps the binding
    // pointing at the current record after each grow.
    let mut a = String::from("abc");
    a.push_str("de");
    println!("{} {} {}", a.len(), a.capacity(), a);
    a.push_str("fghijklmno");
    println!("{} {} {}", a.len(), a.capacity(), a);

    // UTF-8 `push`: one, two and four bytes.
    let mut b = String::new();
    b.push('x');
    b.push('é');
    b.push('😀');
    println!("{} {} {}", b.len(), b.capacity(), b);

    // Comparison against a literal and against another `String`.
    let c = String::from("abcdefghijklmno");
    println!("{} {}", a == "abcdefghijklmno", a == c);
    println!("{} {}", a != c, c == a);

    // The `str` searches read a `String` through `as_str` or directly.
    println!("{} {} {}", a.starts_with("abc"), a.ends_with("mno"), a.contains("def"));
    println!("{}", c.as_str() == "abcdefghijklmno");

    // Growth driven by a loop: capacity doubles from the minimum 8.
    let mut g = String::new();
    for i in 0..10 {
        g.push_str("ab");
        print!("{}:{} ", g.len(), g.capacity());
    }
    println!("");
    println!("{}", g);

    // Growth in a `while` loop with a mixed append.
    let mut h = String::new();
    let mut i = 0;
    while i < 6 {
        h.push_str("xy");
        h.push('!');
        i += 1;
    }
    println!("{} {} {}", h.len(), h.capacity(), h);

    // An empty literal copies to an empty, zero-capacity string.
    let z = String::from("");
    println!("{} {} {}", z.len(), z.capacity(), z.is_empty());
    let mut z2 = String::from("");
    z2.push_str("q");
    println!("{} {} {}", z2.len(), z2.capacity(), z2);
}
