// A destructuring pattern may shadow the very variable it reads, and every
// element read must still see the original value: the expansion copies the
// scrutinee to a hidden temporary first. rustc binds `let (x, y) = x;` from the
// old `x`; a naive expansion that reads through the new `x` would fail to type.
struct S { a: i64, b: i64 }

fn main() {
    // Tuple shadow: the first element rebinds `t`.
    let t: (i64, i64) = (1, 2);
    let (t, u) = t;
    println!("{} {}", t, u);

    // Array shadow.
    let a: [i64; 3] = [3, 4, 5];
    let [a, b, c] = a;
    println!("{} {} {}", a, b, c);

    // Struct shadow: a field rebinds the struct's own name.
    let s = S { a: 10, b: 20 };
    let S { a: s, b } = s;
    println!("{} {}", s, b);

    // Nested tuple shadow with a wildcard-only inner pattern.
    let n: (i64, (i64, i64)) = (7, (8, 9));
    let (n, (_, _)) = n;
    println!("{}", n);

    // `@` binding that shadows its own scrutinee.
    let p: (i64, i64) = (11, 12);
    let q @ (_, _) = p;
    println!("{} {}", q.0, q.1);
}
