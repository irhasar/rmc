// Aggregate constants: a tuple written as `const (1_i32, 2_i32)`, a
// generic-qualified enum variant `const Option::<i32>::Some(1)`, and the
// fieldless `const Option::<i32>::None`. Each must materialise with the same
// layout `rustc -O` gives, and the `while let` must terminate on `None`.
fn main() {
    let t = (1, 2);
    println!("{} {}", t.0, t.1);

    let nested = ((1, 2), 3);
    println!("{}", (nested.0).1);

    let mut o = Some(7);
    while let Some(x) = o {
        println!("{}", x);
        o = None;
    }
    println!("done");

    let n: Option<i32> = None;
    match n {
        Some(x) => println!("some {}", x),
        None => println!("none"),
    }
}
