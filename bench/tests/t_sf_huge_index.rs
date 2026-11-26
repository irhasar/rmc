//! A guarded access at a huge constant index must not crash the compiler.
//!
//! The length is a runtime value, so the bounds comparison cannot be folded and
//! the load at `isize::MAX` survives into codegen as a displacement of
//! `i64::MAX`. Store forwarding keys every access by its byte range, and the end
//! of that range does not exist: the arithmetic must not overflow (the compiler
//! is built with overflow checks), and an access that leaves its slot must keep
//! its pointer key instead of being keyed against the slot.
//!
//! At run time the branch is never taken, so the program must print `n` and exit
//! zero.

use std::hint::black_box;

#[inline(never)]
fn guarded(buf: &mut [u8], i: usize) -> u8 {
    if i < buf.len() {
        buf[i]
    } else {
        0
    }
}

fn main() {
    let mut buf = [7u8; 4];
    let n = black_box(std::env::args().count()).min(buf.len());
    let i = isize::MAX as usize;
    let a = guarded(&mut buf[..n], i);
    let b = guarded(&mut buf[..n], i - 1);
    println!("{a} {b} {n}");
}
