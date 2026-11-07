// Bitwise `& | ^` and shifts `<< >>`, with Rust's precedence: shifts bind
// looser than `+`/`-` and tighter than `&`, which is tighter than `^`, which is
// tighter than `|`. A shift count is taken modulo 64 (hardware masking), and
// `>>` is arithmetic, so a negative value stays negative.
fn main() {
    println!("{}", 0b1100i64 & 0b1010);
    println!("{}", 0b1100i64 | 0b1010);
    println!("{}", 0b1100i64 ^ 0b1010);
    println!("{}", 1i64 << 4);
    println!("{}", 3i64 << 10);
    println!("{}", -16i64 >> 2);
    println!("{}", 16i64 >> 2);

    // Variable shift counts.
    let s: i64 = 3;
    let x: i64 = 5;
    println!("{}", x << s);
    println!("{}", x >> s);

    // Precedence: `+` before `<<`, `&` before `^` before `|`.
    println!("{}", 1i64 + 2 << 3);
    println!("{}", 1i64 & 3 | 4);
    println!("{}", 1i64 ^ 2 & 3);
    println!("{}", 1i64 << 2 + 1);

    // Compound assignment forms.
    let mut m: i64 = 0b1100;
    m &= 0b1010;
    println!("{}", m);
    m |= 0b0001;
    println!("{}", m);
    m ^= 0b1111;
    println!("{}", m);
    m <<= 2;
    println!("{}", m);
    m >>= 1;
    println!("{}", m);

    // A mask in a condition needs no parentheses in Rust either.
    let mut i: i64 = 0;
    let mut even: i64 = 0;
    while i < 16 {
        if i & 1 == 0 {
            even += i;
        }
        i += 1;
    }
    println!("{}", even);
}
