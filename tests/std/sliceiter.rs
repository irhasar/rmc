// `for x in &arr` lowers through `slice::Iter`, whose constructor casts a
// slice pointer (`*mut [T]`) to an element pointer (`*mut T`). rmc holds a
// slice as a `{len, data..}` record, so that cast must skip the length word;
// treated as a no-op it made the first element read the length, shifting every
// element by one. The same loop matches an enum behind the reference, which
// reaches `discriminant((*_1))`. `to_bits` is a transmute, so it must copy the
// bit pattern rather than round the value through a numeric cast.

enum Shape {
    Circle(f64),
    Rect { w: f64, h: f64 },
}

fn area(s: &Shape) -> f64 {
    match s {
        Shape::Circle(r) => 3.0 * *r * *r,
        Shape::Rect { w, h } => *w * *h,
    }
}

fn main() {
    // Integer elements: the length word must not leak into the first read.
    let xs = [10i64, 20, 30, 40];
    for x in &xs {
        println!("{}", *x);
    }

    // Float elements with a runtime accumulator.
    let fs = [1.5f64, 2.5, 3.5];
    let mut t = 0.0f64;
    for f in &fs {
        t += *f;
    }
    println!("{:x}", t.to_bits());

    // An enum behind a reference: `discriminant((*_1))` and a variant field.
    let shapes = [Shape::Circle(2.0), Shape::Rect { w: 3.0, h: 4.0 }];
    let mut total = 0.0f64;
    for s in &shapes {
        total += area(s);
    }
    println!("{:x}", total.to_bits());
}
