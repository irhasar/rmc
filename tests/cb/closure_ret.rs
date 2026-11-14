// Regression: a closure returned from a function and called indirectly. The
// backend route spells the never-called closure value as
// `const ZeroSized: {closure@..}` where `--emit=mir` folds it away.
fn f() -> impl Fn(i64) -> i64 {
    |x| x + 1
}

fn main() {
    println!("{}", f()(41));
}
