// Shared items for the no_std MIR benchmark twins.
//
// Each twin is compiled twice from the same source: once through the rmc back
// end (`rustc --emit=mir -O` then the rmc in-process assembler) and once
// straight through LLVM (`rustc -O`), so the only variable is the back end.
// The programs must therefore be freestanding: no libc, no `std`, and a
// `main` that reads its workload from `argv` (so the optimiser cannot fold it
// away). The twin itself carries `#![no_std]`/`#![no_main]` and includes this
// file for the extern declarations and the panic handler.

use core::panic::PanicInfo;

extern "C" {
    fn print_i64(x: i64);
    fn arg_i64(i: i64) -> i64;
    fn alloc(n: i64) -> i64;
    fn free(p: i64);
    fn print_str(s: *const u8);
    fn putchar(c: i64);
    fn exit(code: i64) -> !;
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

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe { exit(101) }
}
