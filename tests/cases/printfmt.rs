// Differential twin of `printfmt.rms`: exercises `print!`/`println!`, `{}`
// substitution, `{{`/`}}` escapes, the type-directed formatting of `i64`,
// `bool` and `str`, and `println!()` with no arguments.
fn main() {
    println!("hello");
    println!("{}", 42);
    println!("{}", -7);
    println!("{}", true);
    println!("{}", false);
    println!("{}", "text");
    println!("braces {{}}");
    println!("x={} y={}", 1, 2);
    println!("{} and {}", 9, -8);
    print!("no newline");
    println!();
}
