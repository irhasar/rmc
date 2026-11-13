// A `Vec<P>` in the same program as a struct that has a prelude-typed field
// (`Option<i64>`). The size pass must see the prelude's `Option`/`Result` enums
// even though `parse_program` injects them after it runs, or it would fail to
// lay out `Wrapper` and discard *every* struct size — leaving `Vec<P>` with a
// word stride instead of 16. Checked against rustc.
struct Wrapper {
    value: Option<i64>,
}

struct P {
    x: i64,
    y: i64,
}

fn main() {
    let mut v: Vec<P> = Vec::new();
    v.push(P { x: 1, y: 2 });
    v.push(P { x: 3, y: 4 });
    println!("{} {}", v[0].x, v[1].y);

    let w: Wrapper = Wrapper { value: Option::Some(9) };
    match w.value {
        Option::Some(n) => println!("{}", n),
        Option::None => println!("none"),
    }
}
