// A narrow `const` carries its declared width, so a use is substituted at that
// width rather than widened to `i64`. The value folds at its declared type, a
// `const N: u8` used as an array length works, and a narrowing cast in the
// initializer (`300u16 as u8`) is checked at the operand's own width and then
// truncated. Every printed value stays in range, so this runs without the
// overflow checks and still pins the width on the substituted value.

const A: u8 = 5;
const B: i8 = -128;
const C: u16 = 65535;
const D: u8 = 300u16 as u8;
const E: i16 = -32768;
const F: u8 = 250;
const G: u32 = 4294967295;

// A constant initializer that reads an earlier narrow constant keeps its width.
const H: u8 = A;
const I: u8 = A + 1;

fn main() {
    // The value at the declared width.
    println!("{}", A);
    println!("{}", B);
    println!("{}", C);
    println!("{}", E);

    // A narrowing cast folds at the operand's width, then truncates.
    println!("{}", D);

    // A narrow constant used in arithmetic, staying in range.
    println!("{}", F + 5);
    println!("{}", A * 50);
    println!("{}", C - 1);

    // A narrow constant assigned to a binding of the same width, and widened
    // explicitly.
    let x: u8 = A;
    println!("{}", x);
    println!("{}", A as i64 + 100);
    println!("{}", B as i64);
    println!("{}", G as i64);

    // A constant read through another constant.
    println!("{}", H);
    println!("{}", I);

    // A narrow constant as an array length (widened to the index width).
    let arr: [i64; A as usize] = [10, 20, 30, 40, 50];
    println!("{}", arr[A as usize - 1]);

    // A constant in a condition and in a `match` pattern.
    if A < 10 {
        println!("{}", 1);
    }
    match A {
        A => println!("{}", 2),
        _ => println!("{}", 3),
    }

    // A struct field's declared width types an integer literal in its
    // initializer, exactly as a `let` binding does.
    let s = S { a: 200, b: 100 };
    println!("{}", s.a);
    println!("{}", s.b);
    let t = S { a: A, b: C };
    println!("{}", t.a);
    println!("{}", t.b);
}

struct S {
    a: u8,
    b: u16,
}
