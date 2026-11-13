// A `Vec` whose element is a *unit* enum. A unit enum is an eight-byte scalar
// (its tag), not an aggregate, so the element must be widened to a word on
// `push` and read back as a word — passing it as an address (as a struct
// element is) would store a pointer to a temporary and the tag would be lost.
// Checked against rustc.
enum Color {
    Red,
    Green,
    Blue,
}

fn main() {
    let mut v: Vec<Color> = Vec::new();
    v.push(Color::Red);
    v.push(Color::Green);
    v.push(Color::Blue);
    println!("{}", v.len());
    match v[1] {
        Color::Green => println!("green"),
        _ => println!("other"),
    }

    // The same element type through the `vec![..]` list literal.
    let w: Vec<Color> = vec![Color::Blue, Color::Red];
    println!("{}", w.len());
    match w[0] {
        Color::Blue => println!("blue"),
        _ => println!("other"),
    }
}
