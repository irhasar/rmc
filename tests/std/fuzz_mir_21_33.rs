// Found by `tools/fuzz.py --mode mir --seed 21`. The overflow check of
// `v7 / d` folds `assert(!(d == -1 && v7 == i64::MIN))` into the branch and
// drops both comparison statements, but rustc's GVN reuses `v7 == i64::MIN`
// for the overflow check of a later division of `v7`. That later check read a
// `bool` that was never stored, and panicked once the register holding it
// was shared with another value.
use std::hint::black_box;

#[derive(Clone, Copy)]
struct S {
    a: i64,
    b: i64,
}

fn poke(arr: &mut [i64; 7], k: i64) -> i64 {
    let i = (((k % 7) + 7) % 7) as usize;
    arr[i] = -8 - arr[i] * 4;
    arr[(7 - 1 - i) as usize] ^= k;
    arr[i] + k
}

fn spin(n: i64, seed: i64) -> i64 {
    let mut acc: i64 = seed;
    let mut i: i64 = 0;
    while i < n {
        acc = 6 - acc + i * 6;
        i += 1;
    }
    acc
}

fn main() {
    let mut v0: i64 = black_box(3);
    let mut v1: i64 = black_box(517);
    let mut v2: i64 = black_box(583);
    let mut v3: i64 = black_box(-164);
    let mut v4: i64 = black_box(80);
    let mut v5: i64 = black_box(-110);
    let mut v6: i64 = black_box(-924);
    let mut v7: i64 = black_box(757);
    let mut v8: i64 = black_box(558);
    let mut v9: i64 = black_box(-7);
    let mut v10: i64 = black_box(-463);
    let mut arr: [i64; 7] = [black_box(-18); 7];
    let mut bytes: [u8; 8] = [black_box(228u8); 8];
    let guard: [i64; 2] = [black_box(-3); 2];
    let mut s = S { a: v0, b: 5 };
    v2 = (80 + (56 + (v8 % (((v7) & 31) + 1))));
    v6 = (((37 >> (((v0 * -9)) & 15)) % (((((v2 | -1) % ((((v0 + -6)) & 31) + 1))) & 31) + 1)) % (((((v9 ^ 9) & (36 * (v7 ^ 5)))) & 31) + 1));
    {
        let mut i: i64 = 0;
        while i < black_box(1) {
            v6 = -1 - v6;
            i += 1;
        }
    }
    arr[((((((-80 ^ (v6 | -6)) / ((((v9 * (v9 + 4))) & 15) + 1))) % 7) + 7) % 7) as usize] = v10;
    {
        let mut i: i64 = 0;
        while i < black_box(2) {
            v2 = v4 - v2;
            i += 1;
        }
    }
    v4 = poke(&mut arr, (31 + (-62 * (v7 / ((((v5 ^ 9)) & 15) + 1)))));
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
    println!("{}", arr[((((((v1 / (((v3) & 15) + 1)) / ((((v5 | -38)) & 15) + 1))) % 7) + 7) % 7) as usize] + arr[0]);
    println!("{}", bytes[(((((78 * (v7 / (((v3) & 15) + 1)))) % 8) + 8) % 8) as usize] as i64 + bytes[7] as i64);
    println!("{}", guard[0] + guard[1]);
    println!("{} {}", s.a, s.b);
}