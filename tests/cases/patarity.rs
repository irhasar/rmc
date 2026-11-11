// Array and tuple patterns name exactly the scrutinee's element count, and a
// `..` rest absorbs the remaining elements. The bound values must match rustc.
struct S { a: [i64; 2], b: i64 }

fn main() {
    let a: [i64; 3] = [1, 2, 3];
    let [x, y, z] = a;
    println!("{} {} {}", x, y, z);

    let [first, .., last] = a;
    println!("{} {}", first, last);

    let t: (i64, i64, i64) = (10, 20, 30);
    let (p, q, r) = t;
    println!("{} {} {}", p, q, r);

    let (only,) = (7i64,);
    println!("{}", only);

    let () = ();
    println!("unit ok");

    // A nested array inside a struct pattern, with an exact count.
    let s: S = S { a: [4, 5], b: 6 };
    let S { a: [m, n], b } = s;
    println!("{} {} {}", m, n, b);
}
