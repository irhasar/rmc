// An array literal's element width comes from its context, the way a bare
// integer literal takes its width from the surrounding annotation: a `let`
// type, a struct field, a parameter and a `return` type all reach the literal.
// The same context is what lets an empty `[]` be typed at all.

struct Bytes {
    a: [u8; 3],
    n: i64,
}

fn first(a: [u8; 3]) -> i64 {
    a[0] as i64
}

fn make() -> [u8; 3] {
    [7, 8, 9]
}

fn empty() -> [i64; 0] {
    []
}

fn sum(a: [i64; 3]) -> i64 {
    let mut s = 0;
    for x in &a {
        s += *x;
    }
    s
}

fn main() {
    // A `let` annotation, a field, a parameter and a `return` tail.
    let a: [u8; 3] = [1, 2, 3];
    println!("{}", a[0] as i64 + a[2] as i64);
    let b = Bytes { a: [4, 5, 6], n: 10 };
    println!("{}", b.a[1] as i64 + b.n);
    println!("{}", first([11, 12, 13]));
    println!("{}", make()[2] as i64);

    // A nested array literal threads the inner element width too.
    let grid: [[u8; 2]; 2] = [[1, 2], [3, 4]];
    println!("{}", grid[0][0] as i64 + grid[1][1] as i64);

    // An empty array literal is typed by its context, in each position.
    let z: [i64; 0] = [];
    println!("{}", z.len() as i64);
    println!("{}", z.is_empty());
    println!("{}", empty().is_empty());
    let mut acc = 0;
    for x in &z {
        acc += *x;
    }
    println!("{}", acc);

    // A wide array is the default when nothing narrows the width.
    let w = [1000000000000i64, 2, 3];
    println!("{}", sum(w));
    println!("{}", w.is_empty());
}
