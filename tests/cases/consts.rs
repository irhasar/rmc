// Top-level `const` items, folded by the compiler and
// substituted at every use. The rmc and Rust programs must print identical
// output and exit with the same status.
const N: i64 = 5;
const M: i64 = N * 2;
const LIMIT: i64 = M + 1;
const BIG: bool = LIMIT > 10;
const YES: bool = true;
const NEG: i64 = 0 - 3;
// `wrapping_mul` is `*` without the overflow trap, so rustc folds it in a
// `const` item (a plain `*` that overflows is rejected there). The parser sees
// the method-call form, which must fold here as well.
const WRAP: i64 = 9223372036854775807i64.wrapping_mul(2);
const WRAP_SMALL: i64 = 6i64.wrapping_mul(7);
const WRAP_NAMED: i64 = N.wrapping_mul(M);

// An immutable `static` is lowered to a `const`: its initialiser must be a
// constant expression, and every read yields that value. `pub` is accepted.
static S: i64 = N + 1;
pub static S2: i64 = S * 2;

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

    // Constants whose initializer uses `wrapping_mul`.
    println!("{}", WRAP);
    println!("{}", WRAP_SMALL);
    println!("{}", WRAP_NAMED);

    // A `static` behaves exactly like the `const` it lowers to.
    println!("{}", S);
    println!("{}", S2);

    // Block-scoped constants: a `const`/immutable-`static` inside a function
    // body is folded and in scope for the rest of its block, and usable before
    // its declaration (rustc hoists block items). An inner block shadows an
    // outer constant of the same name; the outer value returns when it ends.
    let f = LATER;
    const LOCAL: i64 = 7;
    static LOCAL_STATIC: i64 = 8;
    const LATER: i64 = 9;
    let shadow = {
        const LOCAL: i64 = 100;
        LOCAL
    };
    println!("{}", LOCAL + LOCAL_STATIC + f + shadow);
}
