// Found by `tools/fuzz.py --mode mir` (fuzz-mir-5-184). A scaled-index `lea`
// (`lea 0(,%r15,4), %rax`) was not seen as writing `%rax`, because the
// peepholes split operands at the first comma, so `forward_stores` reused a
// value the `lea` had overwritten.
use std::hint::black_box;

#[derive(Clone, Copy)]
struct S {
    a: i64,
    b: i64,
}

fn poke(arr: &mut [i64; 4], k: i64) -> i64 {
    let i = (((k % 4) + 4) % 4) as usize;
    arr[i] = 4 - arr[i] * 1;
    arr[(4 - 1 - i) as usize] ^= k;
    arr[i] + k
}

fn spin(n: i64, seed: i64) -> i64 {
    let mut acc: i64 = seed;
    let mut i: i64 = 0;
    while i < n {
        acc = 19 - acc + i * 5;
        i += 1;
    }
    acc
}

fn main() {
    let mut v0: i64 = black_box(-355);
    let mut v1: i64 = black_box(268);
    let mut v2: i64 = black_box(-738);
    let mut v3: i64 = black_box(-857);
    let mut v4: i64 = black_box(101);
    let mut v5: i64 = black_box(453);
    let mut v6: i64 = black_box(-693);
    let mut v7: i64 = black_box(300);
    let mut v8: i64 = black_box(808);
    let mut v9: i64 = black_box(-484);
    let mut v10: i64 = black_box(-609);
    let mut arr: [i64; 4] = [black_box(8); 4];
    let mut bytes: [u8; 13] = [black_box(133u8); 13];
    let guard: [i64; 2] = [black_box(0); 2];
    let mut s = S { a: v7, b: 8 };
    v3 = (-88 - (((v9 * -3) / ((((v0 ^ -9)) & 15) + 1)) + (-45 ^ (v9 & 9))));
    {
        let mut i: i64 = 0;
        while i < black_box(2) {
            v5 = (72 - (-2 + (v3 - v10))) - v5;
            i += 1;
        }
    }
    v10 = match ((44 - ((v7 - 18) / (((((v5 - -7) << ((32) & 15))) & 15) + 1)))) & 3 { 0 => (v3 ^ 8), 1 => (v7 ^ -2), 2 => ((48 & (-9 - (v8 ^ 4))) ^ v7), 3 => ((((92 - v9) < ((v2 - 0) / (((v6) & 15) + 1))) as i64) % ((((29 * -27)) & 31) + 1)), _ => (-18 ^ v1) };
    v7 = (-17 + (33 ^ (v9 % ((((v6 & -8)) & 31) + 1))));
    v7 = (v1 * -7);
    v7 = match (((((v8 + -5) > (3 * v3)) as i64) * (v3 ^ 3))) & 3 { 0 => (v6 + -2), 1 => ((((29 % (((v8) & 31) + 1)) / ((((-53 - (v0 | -6))) & 15) + 1)) == ((71 + -32) + (34 - (v7 & -3)))) as i64), _ => (((v2 ^ v9) | ((-39 < 12) as i64)) / (((((50 + v0) % (((-30) & 31) + 1))) & 15) + 1)) };
    arr[((((-49) % 4) + 4) % 4) as usize] = ((-92 ^ ((v1 <= v1) as i64)) % (((-25) & 31) + 1));
    v6 = -35;
    {
        let mut i: i64 = 0;
        while i < black_box(2) {
            v7 = (((-24 - 36) < (95 ^ (-25 - -21))) as i64) - v7;
            i += 1;
        }
    }
    bytes[((((((-34 % (((v0) & 31) + 1)) % ((((v10 / (((v8) & 15) + 1))) & 31) + 1))) % 13) + 13) % 13) as usize] = ((-6 + (-50 * (50 << ((-3) & 15))))) as u8;
    v0 = (-5 * (11 * -16));
    arr[(((((81 * (-37 - v6))) % 4) + 4) % 4) as usize] = v10;
    v4 = (25 - v8);
    bytes[((((((-37 - 13) & (27 - -16))) % 13) + 13) % 13) as usize] = (((((v3 + v8) > ((v6 < 41) as i64)) as i64) / (((v0) & 15) + 1))) as u8;
    println!("{}", v0);
    println!("{}", v1);
    println!("{}", v2);
    println!("{}", v3);
    println!("{}", v4);
    println!("{}", v5);
    println!("{}", v6);
    println!("{}", v7);
    println!("{}", v8);
    println!("{}", v9);
    println!("{}", v10);
    println!("{}", arr[(((((v4 + 9)) % 4) + 4) % 4) as usize] + arr[0]);
    println!("{}", bytes[(((((58 - -26)) % 13) + 13) % 13) as usize] as i64 + bytes[12] as i64);
    println!("{}", guard[0] + guard[1]);
    println!("{} {}", s.a, s.b);
}