// Regression: two or more *runtime* format arguments. rustc builds an
// `Arguments` whose `ArgumentType::Placeholder` carries a zero-sized
// `PhantomData` lifetime; the backend route spells that
// `core::marker::PhantomData`, and treating it as a sized field shifted the
// aggregate and segfaulted on the second argument.
fn main() {
    let x = 1;
    let y = 2;
    println!("{} {}", x, y);
    let a = 3;
    let b = 4;
    let c = 5;
    println!("{} {} {}", a, b, c);
    println!("mix {} then {}", x, "text");
}
