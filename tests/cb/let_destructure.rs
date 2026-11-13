// Regression: array destructuring in a `let` pattern, followed by two runtime
// format arguments.
fn main() {
    let a = [1, 2];
    let [x, y] = a;
    println!("{} {}", x, y);
}
