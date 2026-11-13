// Regression: a unit struct whose value the backend route leaves in the body
// as `_1 = const P;` (the prototype folds it away). Also exercises a `Drop`
// impl and a fieldless enum variant.
struct P;
impl Drop for P {
    fn drop(&mut self) {}
}

enum E {
    A,
    B,
}

fn main() {
    let _p = P;
    let e = E::A;
    match e {
        E::A => println!("a"),
        E::B => println!("b"),
    }
}
