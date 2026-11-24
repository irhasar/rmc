#![no_std]
#![no_main]
include!("prelude.rs");

// A static keeps rustc's packed layout: `([u8; 8], u8)` is 9 bytes, so the
// three elements start at 0, 9 and 18. The back end's tuple layout is
// word-aligned (16 bytes); reading a static through the back end's stride would
// take elements 1 and 2 from the wrong bytes.

static A: [([u8; 8], u8); 3] = [([1; 8], 11), ([2; 8], 22), ([3; 8], 33)];

// Both fields are one byte, so rustc's tuple puts them at 0 and 1 of a 2-byte
// element; the back end's word-per-field layout puts the second at 8.
static B: [(u8, u8); 3] = [(1, 11), (2, 22), (3, 33)];

// A zero-length array is a zero-sized field: rustc's `([i64; 0], u8)` is 8
// bytes (the `u8` at 0), not 16.
static Z: [([i64; 0], u8); 3] = [([], 1), ([], 2), ([], 3)];

// rustc reorders struct fields by alignment: `b` (i64) comes first and `a`
// (u8) sits at byte 8, though the declaration order is the other way round.
// `take` keeps a real `S` aggregate in the body, so the back end learns its
// fields and can address the static.
struct S {
    a: u8,
    b: i64,
}
static D: [S; 3] = [
    S { a: 1, b: 11 },
    S { a: 2, b: 22 },
    S { a: 3, b: 33 },
];

#[inline(never)]
fn take(s: S) -> i64 {
    s.a as i64 + s.b
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1) as usize % 3;
    out(A[k].1 as i64);
    out(A[k].0[0] as i64);
    out(A[k].0[7] as i64);
    out(B[k].1 as i64);
    out(B[k].0 as i64);
    out(Z[k].1 as i64);
    out(take(S { a: k as u8 + 40, b: k as i64 + 50 }));
    out(D[k].a as i64);
    out(D[k].b);
    0
}
