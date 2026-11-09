// Differential twin of `fmtorder.rms`: side-effecting `{}` arguments must run
// before any output, left to right.
fn f() -> &'static str {
    println!("1");
    "ten"
}

fn g() -> &'static str {
    println!("2");
    "twenty"
}

fn main() {
    println!("A{}B{}", f(), g());
}
