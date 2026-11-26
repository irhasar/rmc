//! A guarded access at a huge constant index must not crash the compiler.
//!
//! The length is a runtime value, so the bounds comparison cannot be folded and
//! the load at `isize::MAX` survives into codegen. A temporary slice keeps its
//! data pointer a plain `stack_addr`, so the load lands at
//! `stack_addr + i64::MAX`: the end of that range does not exist. Store
//! forwarding keys every access by its byte range, so it must not compute that
//! end with wrapping arithmetic (the end would come out negative and pass the
//! upper-bound test) nor with overflow checks on (the compiler would abort).
//!
//! At run time the branch is never taken, so the program prints `0 0 1`.

use std::hint::black_box;

fn main() {
    let mut arr = [7u8; 4];
    let n = black_box(std::env::args().count()).min(4);
    let huge = isize::MAX as usize;
    let a = if huge < (&mut arr[..n]).len() {
        (&mut arr[..n])[huge]
    } else {
        0
    };
    let b = if huge - 1 < (&mut arr[..n]).len() {
        (&mut arr[..n])[huge - 1]
    } else {
        0
    };
    println!("{a} {b} {n}");
}
