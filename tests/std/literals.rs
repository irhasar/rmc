// Format arguments written as literals directly in the placeholder, which
// rustc promotes into a hidden `main::promoted[N]` static. The back end folds
// the promoted body into a `.rodata` blob, so these must match `rustc -O`.
fn main() {
    // Integer literals in every base.
    println!("{:x}", 255);
    println!("{:X}", 255);
    println!("{:o}", 8);
    println!("{:b}", 5);
    println!("{}", 42);
    // A negative literal keeps its bit pattern.
    println!("{:x}", -1i32);

    // `bool` literals.
    println!("{} {}", true, false);
    println!("{:?}", true);

    // `char` literals, including multi-byte UTF-8.
    println!("{}", 'A');
    println!("{}", 'é');
    println!("{}", '中');
    println!("{}", '\n');
    println!("{}", '\t');
    println!("{}", '\u{1F600}');

    // Several promoted arguments in one call.
    println!("{} {} {}", true, 'x', 7);

    // A promoted literal mixed with a runtime value.
    let n = 200;
    println!("{} {:x} {}", n, n, 'z');
}
