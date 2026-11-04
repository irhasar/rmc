// Rust twin of consts.rms. Top-level `const` items, folded by the compiler and
// substituted at every use. The rmc and Rust programs must print identical
// output and exit with the same status.
const N: i64 = 5;
const M: i64 = N * 2;
const LIMIT: i64 = M + 1;
const BIG: bool = LIMIT > 10;
const YES: bool = true;
const NEG: i64 = 0 - 3;

fn main() {
    // A constant in expression position.
    println!("{}", N);
    println!("{}", M);
    println!("{}", LIMIT);
    println!("{}", NEG);

    // A constant as an array length, and as an array repeat count.
    let a: [i64; N as usize] = [1, 2, 3, 4, 5];
    let b: [i64; N as usize] = [7; N as usize];
    println!("{}", a[0] + a[N as usize - 1]);
    println!("{}", b[4]);

    // A constant used in a `for` bound.
    let mut s: i64 = 0;
    for i in 0..N {
        s = s + a[i as usize];
    }
    println!("{}", s);

    // A constant in a condition and in a `match` pattern.
    if BIG { println!("{}", 100); }
    if YES { println!("{}", 200); }
    match N {
        N => { println!("{}", 300); }
        _ => { println!("{}", 301); }
    }
    match NEG {
        NEG => { println!("{}", 400); }
        _ => { println!("{}", 401); }
    }
    // A constant in a value-position `if`.
    let v: i64 = if BIG { M } else { 0 };
    println!("{}", v);
}
