// A `Vec` whose element is a struct: the element is stored inline (its whole
// size is the stride, not a word), so `Vec::new()`+`push`, the `vec![..]` list
// and `vec![v; n]` repeat literals, index read/write, `.iter()` and a by-value
// `for` all move whole structs. Every path is checked against rustc.
#[derive(Copy, Clone)]
struct P {
    x: i64,
    y: i64,
}

#[derive(Copy, Clone)]
struct Q {
    a: i64,
}

fn main() {
    // `Vec::new()` + `push`: the store starts empty and grows; each element is
    // the full 16-byte struct.
    let mut v: Vec<P> = Vec::new();
    for i in 0..5 {
        v.push(P { x: i, y: i * 10 });
    }
    println!("{} {}", v.len(), v.capacity());
    for i in 0..v.len() {
        print!("{}:{} ", v[i].x, v[i].y);
    }
    println!("");

    // Index write: assign a whole struct through `v[i]`.
    v[0] = P { x: 100, y: 200 };
    println!("{} {}", v[0].x, v[0].y);
    // Field write through the index.
    v[1].y = -7;
    println!("{} {}", v[1].x, v[1].y);

    // `vec![..]` list literal: exactly sized, capacity equals length.
    let l: Vec<P> = vec![P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];
    println!("{} {}", l.len(), l.capacity());
    for i in 0..l.len() {
        print!("{}:{} ", l[i].x, l[i].y);
    }
    println!("");

    // `vec![v; n]` repeat literal: `n` copies of one struct.
    let r: Vec<P> = vec![P { x: 9, y: 8 }; 4];
    println!("{} {}", r.len(), r.capacity());
    for i in 0..r.len() {
        print!("{}:{} ", r[i].x, r[i].y);
    }
    println!("");

    // A single-field struct is a word wide but still an aggregate, so it must
    // be copied by value and not stored as its address.
    let s: Vec<Q> = vec![Q { a: 11 }, Q { a: 22 }];
    println!("{} {}", s[0].a, s[1].a);
    let mut m: Vec<Q> = Vec::new();
    m.push(Q { a: 33 });
    m.push(Q { a: 44 });
    println!("{} {}", m[0].a, m[1].a);

    // `.iter()` borrows each element; a by-value `for` moves each element out.
    let mut sum = 0;
    for p in l.iter() {
        sum += p.x + p.y;
    }
    println!("{}", sum);
    let mut sum2 = 0;
    for p in r {
        sum2 += p.x * p.y;
    }
    println!("{}", sum2);

    // `&mut` iteration writes through to the store.
    let mut w: Vec<P> = vec![P { x: 1, y: 1 }, P { x: 2, y: 2 }];
    for p in w.iter_mut() {
        p.x += 10;
    }
    for i in 0..w.len() {
        print!("{}:{} ", w[i].x, w[i].y);
    }
    println!("");

    // The default element type is inferred from a struct-literal list.
    let d = vec![P { x: 7, y: 7 }, P { x: 8, y: 8 }];
    println!("{} {}", d[0].x, d[1].y);
}
