// Range adapters: `.step_by(k)`, `.skip(k)` and `.take(k)`, and their
// composition with each other and with `.rev()`. Each adapter transforms the
// previous one's sequence, so the chains below must match rustc exactly. The
// bounds are literals because rustc's `.take(..).rev()` needs an
// `ExactSizeIterator`, which `Range<i64>` is not (the literals default to i32
// there; rmc is i64 throughout and accepts both).
fn main() {
    print!("step_by: ");
    for i in (0..10).step_by(3) { print!("{} ", i); }
    println!();

    print!("skip: ");
    for i in (0..10).skip(2) { print!("{} ", i); }
    println!();

    print!("take: ");
    for i in (0..10).take(4) { print!("{} ", i); }
    println!();

    print!("skip+take: ");
    for i in (0..10).skip(2).take(3) { print!("{} ", i); }
    println!();

    print!("take+rev: ");
    for i in (0..10).take(4).rev() { print!("{} ", i); }
    println!();

    print!("skip+take+rev: ");
    for i in (0..10).skip(2).take(3).rev() { print!("{} ", i); }
    println!();

    print!("step_by+rev: ");
    for i in (0..10).step_by(3).rev() { print!("{} ", i); }
    println!();

    print!("rev+take: ");
    for i in (0..10).rev().take(3) { print!("{} ", i); }
    println!();

    print!("inclusive step_by: ");
    for i in (0..=10).step_by(4) { print!("{} ", i); }
    println!();

    print!("inclusive rev: ");
    for i in (0..=10).rev() { print!("{} ", i); }
    println!();

    print!("offset skip: ");
    for i in (3..20).skip(2) { print!("{} ", i); }
    println!();

    print!("overshoot skip: ");
    for i in (0..5).skip(10) { print!("{} ", i); }
    println!("empty");

    print!("overshoot take: ");
    for i in (0..3).take(10) { print!("{} ", i); }
    println!();

    print!("empty step_by: ");
    for i in (5..5).step_by(2) { print!("{} ", i); }
    println!("empty");

    print!("continue: ");
    for i in (0..10).skip(1).take(5) {
        if i == 3 { continue; }
        print!("{} ", i);
    }
    println!();

    print!("step_by k=1: ");
    for i in (0..4).step_by(1) { print!("{} ", i); }
    println!();
}
