#![no_std]
#![no_main]
include!("prelude.rs");

// `[v; n]` must store `v` with the element's real size. The lowering stored
// one 8-byte word per element with an 8-byte stride whatever the element was:
// `[0u8; N]` wrote 8*N bytes (running off the frame for a large N), and a
// multi-word element was initialised in its first word only. The reads use a
// runtime index, or rustc's GVN replaces `a[k]` with the repeated operand and
// never touches the array.
#[derive(Clone, Copy)]
struct P {
    a: i64,
    b: i64,
    c: i64,
}

#[inline(never)]
fn bytes(k: i64, i: usize) -> i64 {
    // A byte array followed by a guard word that an 8-byte stride overwrites.
    let mut guard: [i64; 2] = [k; 2];
    let b: [u8; 24] = [7u8; 24];
    guard[1] += b[i] as i64;
    guard[0] + guard[1]
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let i = arg(2) as usize;
    let ps: [P; 6] = [P { a: k, b: k + 1, c: k + 2 }; 6];
    out(ps[i].a + ps[i].b + ps[i].c);
    out(ps[5 - i].c);
    let m: [[i64; 3]; 4] = [[k; 3]; 4];
    out(m[i % 4][i % 3]);
    out(bytes(k, i));
    let w: [i32; 9] = [k as i32; 9];
    out(w[i] as i64);
    0
}
