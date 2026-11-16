#![no_std]
#![no_main]
include!("prelude.rs");

// `x = if c { A } else { B }` with constant arms becomes straight-line code
// (`select_diamonds`): `c as T`, `(c as T) ^ 1`, or `(c as T) * (A - B) + B`.
// Each shape below has a condition that varies at run time (`arg`, and a
// pseudo-random stream in a loop), and a destination of each type the pass
// handles or must leave alone (`bool`, `u8`, `i32`, `u64`, `i64`).

#[inline(never)]
fn one_zero(k: i64) -> i64 {
    let x: i64 = if k > 3 { 1 } else { 0 };
    x * 100 + k
}

#[inline(never)]
fn zero_one(k: i64) -> i64 {
    let x: i64 = if k > 3 { 0 } else { 1 };
    x * 100 + k
}

#[inline(never)]
fn two_constants(k: i64) -> i64 {
    let x: i64 = if k & 1 == 0 { 7 } else { -3 };
    let y: i64 = if k < 0 { i64::MIN } else { i64::MAX };
    x.wrapping_mul(1000).wrapping_add(y)
}

#[inline(never)]
fn wide_unsigned(k: i64) -> i64 {
    let x: u64 = if k > 0 { u64::MAX } else { 3 };
    (x / 2) as i64
}

#[inline(never)]
fn narrow(k: i64) -> i64 {
    let a: u8 = if k > 5 { 1 } else { 0 };
    let b: u8 = if k > 5 { 0 } else { 1 };
    // Not 0/1: kept as a branch (narrow arithmetic is not exact here).
    let c: u8 = if k > 5 { 200 } else { 17 };
    let d: i32 = if k > 5 { -40000 } else { 70000 };
    a as i64 + 10 * b as i64 + 100 * c as i64 + d as i64
}

#[inline(never)]
fn flags(k: i64) -> i64 {
    let p: bool = if k > 2 { false } else { true };
    let q: bool = if k > 2 { true } else { false };
    let r: bool = if k > 2 { true } else { true };
    (p as i64) + 2 * (q as i64) + 4 * (r as i64)
}

// The `bfs` shape: a data-dependent pick inside a loop, stored to an array.
#[inline(never)]
fn stream(seed: i64, n: i64) -> i64 {
    let mut s = seed | 1;
    let mut cells = [0i64; 64];
    let mut i = 0;
    while i < n {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let v = if (s >> 1) % 100 < 28 { 5 } else { -2 };
        cells[(i & 63) as usize] += v;
        i += 1;
    }
    let mut t = 0;
    let mut j = 0;
    while j < 64 {
        t = t * 31 + cells[j];
        j += 1;
    }
    t
}

// A short-circuit chain ending in a select (`life`'s rule) folds into one
// condition, then into arithmetic. Every comparison kind and both `&&` and
// `||`, on integer and `bool` discriminants.
#[inline(never)]
fn chains(k: i64, m: i64) -> i64 {
    let mut t = 0;
    let mut s = k - 6;
    while s < k + 6 {
        let alive = (s ^ m) & 1;
        let next: i64 = if s == 3 || (s == 2 && alive == 1) { 1 } else { 0 };
        let both: i64 = if s > m && alive != 0 { 9 } else { -4 };
        let either: u8 = if s < 0 || m < s { 2 } else { 250 };
        let flag = s >= m;
        let mixed: bool = if flag && !(alive == 0) { true } else { false };
        t = t * 7 + next * 1000 + both * 100 + either as i64 * 10 + mixed as i64;
        s += 1;
    }
    t
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    out(one_zero(k));
    out(zero_one(k));
    out(two_constants(k));
    out(wide_unsigned(k));
    out(narrow(k));
    out(flags(k));
    out(stream(k, 1000));
    out(chains(k, 1));
    out(chains(k, -k));
    0
}
