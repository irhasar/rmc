// `&`, `|` and `^` are defined on `bool` (rustc's `BitAnd`/`BitOr`/`BitXor`
// impls), not only on `i64`. A `bool` is a 0/1 word, so the bitwise instruction
// is already the logical result; the twin prints the value `as i64` so both
// compilers write the same bytes.
const CONJ: bool = true & false;
const DISJ: bool = false | true;
const XORC: bool = true ^ true;

fn and(a: bool, b: bool) -> bool { a & b }
fn or(a: bool, b: bool) -> bool { a | b }
fn xor(a: bool, b: bool) -> bool { a ^ b }

fn main() {
    println!("{}", (true & true) as i64);
    println!("{}", (true & false) as i64);
    println!("{}", (false | true) as i64);
    println!("{}", (false | false) as i64);
    println!("{}", (true ^ false) as i64);
    println!("{}", (true ^ true) as i64);

    let a: bool = 1 < 2;
    let b: bool = 3 < 2;
    println!("{}", (a & b) as i64);
    println!("{}", (a | b) as i64);
    println!("{}", (a ^ b) as i64);
    println!("{}", (b ^ b) as i64);

    // Precedence: `&` binds tighter than `|`, which binds tighter than `^`,
    // exactly as for the integers.
    println!("{}", (true | false & false) as i64);
    println!("{}", (true ^ false | false) as i64);

    // Compound assignment on a `bool`.
    let mut c: bool = true;
    c &= false;
    println!("{}", c as i64);
    c |= true;
    println!("{}", c as i64);
    c ^= true;
    println!("{}", c as i64);

    // A `const bool` folded from the operators.
    println!("{}", CONJ as i64);
    println!("{}", DISJ as i64);
    println!("{}", XORC as i64);

    // Through calls and as an `if` condition.
    println!("{}", and(a, b) as i64);
    println!("{}", or(b, a) as i64);
    println!("{}", xor(a, a) as i64);
    if a & true {
        println!("{}", 1);
    } else {
        println!("{}", 0);
    }
    let arr: [bool; 3] = [true & false, true | false, true ^ true];
    println!("{} {} {}", arr[0] as i64, arr[1] as i64, arr[2] as i64);
}
