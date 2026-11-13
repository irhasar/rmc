// A `Vec` whose element is a struct with a *non-word* field. rmc stores a
// scalar field in one 8-byte slot, so a struct's size is the sum of its field
// sizes with no padding: a `u8` field is one byte, an `[i64; 2]` field is two
// words, a tuple or enum field is the aggregate's own size. The element stride
// is that size, so a wrong layout silently mis-scales the store and corrupts
// the next element. Every case is checked against rustc.

enum E {
    A(i64),
    B(i64),
}

struct Inner {
    p: i64,
    q: i64,
}

// A byte field before a word field: the byte is one byte, not a whole word.
struct Bytes {
    x: u8,
    y: i64,
}

// An array field, whose stride must count both elements.
struct WithArr {
    arr: [i64; 2],
    t: i64,
}

// A tuple field (lowered to an anonymous struct) and a nested struct field.
struct WithTuple {
    p: (i64, i64),
    t: i64,
}

struct WithEnum {
    e: E,
    t: i64,
}

struct Nested {
    p: Inner,
    q: i64,
}

fn main() {
    let mut b: Vec<Bytes> = Vec::new();
    b.push(Bytes { x: 7, y: 9 });
    b.push(Bytes { x: 8, y: 10 });
    println!("{} {} {} {}", b[0].x, b[0].y, b[1].x, b[1].y);

    let a: Vec<WithArr> = vec![
        WithArr { arr: [1, 2], t: 3 },
        WithArr { arr: [4, 5], t: 6 },
    ];
    println!("{} {} {} {} {} {}", a[0].arr[0], a[0].arr[1], a[0].t, a[1].arr[0], a[1].arr[1], a[1].t);

    let t: Vec<WithTuple> = vec![
        WithTuple { p: (1, 2), t: 3 },
        WithTuple { p: (4, 5), t: 6 },
    ];
    println!("{} {} {} {} {} {}", t[0].p.0, t[0].p.1, t[0].t, t[1].p.0, t[1].p.1, t[1].t);

    let mut e: Vec<WithEnum> = Vec::new();
    e.push(WithEnum { e: E::A(1), t: 2 });
    e.push(WithEnum { e: E::B(3), t: 4 });
    match e[0].e {
        E::A(n) => println!("A {} {}", n, e[0].t),
        E::B(n) => println!("B {} {}", n, e[0].t),
    }
    match e[1].e {
        E::A(n) => println!("A {} {}", n, e[1].t),
        E::B(n) => println!("B {} {}", n, e[1].t),
    }

    let n: Vec<Nested> = vec![
        Nested { p: Inner { p: 1, q: 2 }, q: 3 },
        Nested { p: Inner { p: 4, q: 5 }, q: 6 },
    ];
    println!("{} {} {} {} {} {}", n[0].p.p, n[0].p.q, n[0].q, n[1].p.p, n[1].p.q, n[1].q);
}
