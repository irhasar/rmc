// Found by `tools/fuzz.py --mode mir` (fuzz-mir-4-93). A scaled-index `lea`
// (`lea 0(,%r15,4), %rax`) was not seen as writing `%rax`, because the
// peepholes split operands at the first comma, so `forward_stores` reused a
// value the `lea` had overwritten.
use std::hint::black_box;

#[derive(Clone, Copy)]
struct S {
    a: i64,
    b: i64,
}

fn poke(arr: &mut [i64; 8], k: i64) -> i64 {
    let i = (((k % 8) + 8) % 8) as usize;
    arr[i] = 8 - arr[i] * 4;
    arr[(8 - 1 - i) as usize] ^= k;
    arr[i] + k
}

fn spin(n: i64, seed: i64) -> i64 {
    let mut acc: i64 = seed;
    let mut i: i64 = 0;
    while i < n {
        acc = 34 - acc + i * 6;
        i += 1;
    }
    acc
}

fn main() {
    let mut v0: i64 = black_box(868);
    let mut v1: i64 = black_box(-716);
    let mut v2: i64 = black_box(-509);
    let mut v3: i64 = black_box(-133);
    let mut v4: i64 = black_box(170);
    let mut v5: i64 = black_box(318);
    let mut v6: i64 = black_box(134);
    let mut v7: i64 = black_box(964);
    let mut v8: i64 = black_box(836);
    let mut v9: i64 = black_box(298);
    let mut v10: i64 = black_box(431);
    let mut arr: [i64; 8] = [black_box(16); 8];
    let mut bytes: [u8; 3] = [black_box(169u8); 3];
    let guard: [i64; 2] = [black_box(2); 2];
    let mut s = S { a: v7, b: -2 };
    v4 = spin(black_box(1), (((26 & v10) % ((((84 - v1)) & 31) + 1)) * (4 * v10)));
    v9 = (-79 - (-58 - ((v5 + 4) - v6)));
    v8 = match (((((4 / (((-38) & 15) + 1)) / ((((-85 - -13)) & 15) + 1)) < ((-4 ^ v10) / (((v3) & 15) + 1))) as i64)) & 3 { 0 => (0 - v10), 1 => v0, _ => (v9 / ((((((v3 * 3) % (((v2) & 31) + 1)) | (-48 - 34))) & 15) + 1)) };
    v9 = (-27 - (v10 - 4));
    v7 = (54 - (-88 - ((v7 | 0) | v2)));
    v0 = v2;
    {
        let mut i: i64 = 0;
        while i < black_box(5) {
            v7 = (((((v6 != (v10 ^ -4)) as i64) >> (((v7 % (((34) & 31) + 1))) & 15)) < (-23 ^ 24)) as i64) - v7;
            i += 1;
        }
    }
    v9 = v9;
    v1 = (17 - ((1 - v5) % ((((v1 & 0)) & 31) + 1)));
    v3 = ((-11 - (-64 * v3)) % ((((v6 * 6)) & 31) + 1));
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
    println!("{}", arr[((((v7) % 8) + 8) % 8) as usize] + arr[0]);
    println!("{}", bytes[(((((87 * (-94 - (v8 - -9)))) % 3) + 3) % 3) as usize] as i64 + bytes[2] as i64);
    println!("{}", guard[0] + guard[1]);
    println!("{} {}", s.a, s.b);
}