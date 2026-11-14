// Shared prelude for the MIR differential cases.
//
// Each case is a freestanding `#![no_std]` program that includes this file.
// The same source is compiled twice: by rustc to an object (the reference) and
// by rmc's rustc-MIR front end. Both are linked against the *same* freestanding
// runtime, so the two binaries differ only in the code generator and must print
// identical output and exit with the same status.
extern "C" {
    fn print_i64(x: i64);
    fn print_bool(b: i64);
    fn arg_i64(i: i64) -> i64;
    fn alloc(n: i64) -> i64;
    fn free(p: i64);
    fn putchar(c: i64);
    fn exit(c: i64) -> !;
}

#[inline(always)]
fn out(x: i64) {
    unsafe { print_i64(x) }
}
#[inline(always)]
fn outb(b: bool) {
    unsafe { print_bool(b as i64) }
}
#[inline(always)]
fn arg(i: i64) -> i64 {
    unsafe { arg_i64(i) }
}
#[inline(always)]
fn alc(n: i64) -> i64 {
    unsafe { alloc(n) }
}
#[inline(always)]
fn rel(p: i64) {
    unsafe { free(p) }
}

#[panic_handler]
fn panic(_i: &core::panic::PanicInfo) -> ! {
    unsafe { exit(101) }
}
