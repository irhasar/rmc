//! Shared preamble for the no_std MIR benchmark twins.
//!
//! Each program is compiled with `rustc --emit=mir -O` and then lowered by the
//! rmc backend, so it must be freestanding: no libc, no `std`, and a `main`
//! that reads its workload from `argv` (so the optimiser cannot fold it away).
#![no_std]
#![no_main]
#![allow(dead_code)]

extern "C" {
    fn print_i64(x: i64);
    fn arg_i64(i: i64) -> i64;
    fn alloc(n: i64) -> i64;
    fn print_str(s: *const u8);
    fn putchar(c: i64);
}

#[inline(always)]
fn print(x: i64) {
    unsafe { print_i64(x) }
}

#[inline(always)]
fn arg(i: i64) -> i64 {
    unsafe { arg_i64(i) }
}

#[inline(always)]
fn alc(n: i64) -> i64 {
    unsafe { alloc(n) }
}
