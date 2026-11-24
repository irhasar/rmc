#![no_std]
#![no_main]
include!("prelude.rs");

// `black_box::<T>` for a one-word `T` is the runtime's identity call. Besides
// the integers, a float travels in a general-purpose register and a reference to
// a fixed-size array is a thin pointer, so both are word-sized. A `&str` or a
// `&[T]` is two words in rustc and stays refused (see `reject/`).
//
// `Instruction` has the letters `str` in its name: a `&Instruction` is a thin
// pointer, and a type test that looks for the substring refuses it.

struct Instruction {
    op: i64,
}

#[inline(never)]
fn floats(x: f64, y: f32) -> i64 {
    let a = core::hint::black_box(x * 1.5);
    let b = core::hint::black_box(y + 0.25);
    (a * 100.0) as i64 + (b * 10.0) as i64
}

#[inline(never)]
fn array_ref(t: &[i64; 4], i: usize) -> i64 {
    let r = core::hint::black_box(t);
    r[i % 4]
}

#[inline(never)]
fn array_mut(t: &mut [u8; 3], i: usize) -> i64 {
    let r = core::hint::black_box(t);
    r[i % 3] = r[i % 3].wrapping_add(7);
    r[0] as i64 + r[1] as i64 * 10 + r[2] as i64 * 100
}

#[inline(never)]
fn named(ins: &Instruction, k: i64) -> i64 {
    core::hint::black_box(ins).op + k
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    out(floats(k as f64, k as f32));
    let t = [k, k + 1, k + 2, k + 3];
    out(array_ref(&t, k as usize));
    let mut u = [1u8, 2, 3];
    out(array_mut(&mut u, k as usize));
    out(named(&Instruction { op: k * 5 }, 9));
    0
}
