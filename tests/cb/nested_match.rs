// Regression: a tuple-plus-`Option` match whose arms print two runtime values.
fn main() {
    let t = (1, Some(2));
    match t {
        (a, Some(b)) => println!("{} {}", a, b),
        _ => {}
    }
}
