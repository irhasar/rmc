// A method argument that is a `loop` expression can assign the receiver. Rust
// evaluates and copies the receiver before the argument, so with `x = 2`,
// `x.min(loop { x = 8; break 3; })` is `2.min(3) == 2`. If the receiver is not
// snapshotted, the desugared comparison reads the mutated `x` and returns 3.
fn main() {
    let mut x: i64 = 2;
    let result = x.min(loop { x = 8; break 3; });
    println!("{}", result);

    // The same for an argument that assigns through a nested block, and for a
    // loop whose value is used by a different method.
    let mut y: i64 = 10;
    let r2 = y.max({ y = 100; 7 });
    println!("{}", r2);

    let mut z: i64 = 3;
    let r3 = z.min(loop { z = z + 1; break 1; });
    println!("{}", r3);
}
