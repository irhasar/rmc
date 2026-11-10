// Struct field promotion. A hot scalar field is kept in a register, but a
// struct that is ever copied, returned or passed by value keeps its fields in
// memory. Both paths must produce identical results.
#[derive(Clone, Copy)]
struct Vec2 { x: i64, y: i64 }
#[derive(Clone, Copy)]
struct Big { a: i64, b: i64, c: i64, d: i64 }
#[derive(Clone, Copy)]
struct Inner { p: i64, q: i64 }
#[derive(Clone, Copy)]
struct Outer { inner: Inner, z: i64 }
#[derive(Clone, Copy)]
struct WithArr { tag: i64, data: [i64; 3] }

fn addv(p: Vec2, q: Vec2) -> Vec2 {
    Vec2 { x: p.x + q.x, y: p.y + q.y }
}

fn sum_big(b: Big) -> i64 {
    b.a + b.b + b.c + b.d
}

fn make(x: i64) -> Vec2 {
    Vec2 { x: x, y: x * 2 }
}

fn main() {
    let mut v: Vec2 = Vec2 { x: 1, y: 2 };
    let mut i: i64 = 0;
    while i < 5 {
        v.x = v.x + v.y;
        v.y = v.y + 1;
        i += 1;
    }
    println!("{}", v.x);
    println!("{}", v.y);

    let a: Vec2 = Vec2 { x: 10, y: 20 };
    let mut b: Vec2 = a;
    b.x = 99;
    println!("{}", a.x);
    println!("{}", b.x);
    println!("{}", a.y);

    let s: Vec2 = addv(a, b);
    println!("{}", s.x);
    println!("{}", s.y);
    let r: Vec2 = make(7);
    println!("{}", r.x);
    println!("{}", r.y);

    println!("{}", sum_big(Big { a: 1, b: 2, c: 3, d: 4 }));
    let mut m: Vec2 = Vec2 { x: 0, y: 3 };
    while m.x < m.y {
        m.x += 1;
    }
    println!("{}", m.x);

    let w = Vec2 { x: 5, y: 6 };
    println!("{}", w.x + w.y);

    let mut o: Outer = Outer { inner: Inner { p: 1, q: 2 }, z: 3 };
    o.inner.p = o.inner.p + o.inner.q;
    println!("{}", o.inner.p);
    println!("{}", o.inner.q);
    println!("{}", o.z);

    let mut wa: WithArr = WithArr { tag: 4, data: [1, 2, 3] };
    wa.data[1] = wa.data[0] + wa.data[2];
    println!("{}", wa.tag);
    println!("{}", wa.data[0]);
    println!("{}", wa.data[1]);
    println!("{}", wa.data[2]);
}
