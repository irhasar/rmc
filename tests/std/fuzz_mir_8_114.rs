// Found by `tools/fuzz.py --mode mir` (fuzz-mir-8-114). A scaled-index `lea`
// (`lea 0(,%r15,4), %rax`) was not seen as writing `%rax`, because the
// peepholes split operands at the first comma, so `forward_stores` reused a
// value the `lea` had overwritten.
use std::hint::black_box;

#[derive(Clone, Copy)]
struct S {
    a: i64,
    b: i64,
}

fn poke(arr: &mut [i64; 3], k: i64) -> i64 {
    let i = (((k % 3) + 3) % 3) as usize;
    arr[i] = -7 - arr[i] * 1;
    arr[(3 - 1 - i) as usize] ^= k;
    arr[i] + k
}

fn spin(n: i64, seed: i64) -> i64 {
    let mut acc: i64 = seed;
    let mut i: i64 = 0;
    while i < n {
        acc = 4 - acc + i * 2;
        i += 1;
    }
    acc
}

fn main() {
    let mut v0: i64 = black_box(779);
    let mut v1: i64 = black_box(-664);
    let mut v2: i64 = black_box(-96);
    let mut v3: i64 = black_box(132);
    let mut v4: i64 = black_box(-275);
    let mut v5: i64 = black_box(-836);
    let mut arr: [i64; 3] = [black_box(5); 3];
    let mut bytes: [u8; 5] = [black_box(113u8); 5];
    let guard: [i64; 2] = [black_box(-3); 2];
    let mut s = S { a: v3, b: 6 };
    v1 = (v1 & -1);
    v5 = (-13 % (((((42 % (((17) & 31) + 1)) % (((v0) & 31) + 1))) & 31) + 1));
    {
        let mut i: i64 = 0;
        while i < black_box(4) {
            v1 = v5 - v1;
            i += 1;
        }
    }
    v3 = spin(black_box(4), v0);
    s.a = (26 + (-100 - (-11 - 40))) - s.a;
    v4 = (-35 - (96 * (-23 / (((15) & 15) + 1))));
    v2 = ((75 * (v1 * (v1 ^ 4))) / ((((((v1 | 4) / ((((v5 & -6)) & 15) + 1)) ^ v1)) & 15) + 1));
    v1 = spin(black_box(2), (75 - (51 + 14)));
    arr[((((-8) % 3) + 3) % 3) as usize] = (-58 - ((45 << ((-15) & 15)) / (((2) & 15) + 1)));
    v5 = (((((v0 & 6) * v4) <= (-83 * -32)) as i64) * (9 * v5));
    arr[((((((30 ^ (v0 * -7)) << ((((v4 | -3) / (((v4) & 15) + 1))) & 15))) % 3) + 3) % 3) as usize] = ((((v2 * 6) / ((((30 ^ v1)) & 15) + 1)) == (-96 * v4)) as i64);
    s.a = ((-33 - (-46 * v0)) >> ((7) & 15)) - s.b;
    println!("{}", v0);
    println!("{}", v1);
    println!("{}", v2);
    println!("{}", v3);
    println!("{}", v4);
    println!("{}", v5);
    println!("{}", arr[(((((29 - ((v3 - -9) / (((48) & 15) + 1)))) % 3) + 3) % 3) as usize] + arr[0]);
    println!("{}", bytes[((((((-1 * v5) % (((v2) & 31) + 1))) % 5) + 5) % 5) as usize] as i64 + bytes[4] as i64);
    println!("{}", guard[0] + guard[1]);
    println!("{} {}", s.a, s.b);
}