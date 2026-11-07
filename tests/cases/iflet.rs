// `if let PAT = EXPR { then } else { els }`, Rust's sugar for
// `match EXPR { PAT => then, _ => els_or_unit }`. Unlike `match`, it need not
// be exhaustive. Exercised here with an integer, `true`, a binding, an enum
// payload binding, a `else if let` chain, a side-effecting scrutinee evaluated
// once, and a value-position `if let`.
#[derive(Copy, Clone)]
enum Shape { Dot, Line(i64), Rect(i64, i64) }

fn side(k: i64) -> i64 {
    println!("{}", k);
    k + 1
}

fn area(s: Shape) -> i64 {
    if let Shape::Rect(w, h) = s {
        w * h
    } else if let Shape::Line(w) = s {
        w
    } else {
        0
    }
}

fn main() {
    let o: i64 = 3;
    if let 3 = o { println!("{}", 10); } else { println!("{}", 11); }
    if let 4 = o { println!("{}", 12); } else { println!("{}", 13); }
    // A binding pattern always matches and binds the scrutinee.
    if let y = o { println!("{}", y * 2); }
    // The scrutinee is a call: it must run exactly once.
    if let 8 = side(7) { println!("{}", 1); } else { println!("{}", 0); }
    if let true = o < 5 { println!("{}", 2); } else { println!("{}", 3); }
    println!("{}", area(Shape::Rect(3, 4)));
    println!("{}", area(Shape::Line(9)));
    println!("{}", area(Shape::Dot));
    // Value position: both arms must have the same type.
    let v: i64 = if let 3 = o { 100 } else { 200 };
    println!("{}", v);
}
