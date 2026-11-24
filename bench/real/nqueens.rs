// A `bench/real` workload: realistic code that was *not* used to tune rmc,
// so it checks whether a speed-up seen on `bench/uni` carries over (see
// `bench/run_four.py --set real`). The same program is a regression case in
// `tests/mir/real_*.rs`. The prelude below is the shared `bench/uni` one.

#![no_std]
#![no_main]

// One source, four back ends: rmc's AST front end, and rustc driving rmc's
// codegen backend, rustc's Cranelift backend, and rustc's LLVM backend.
//
// The runtime declarations and the panic handler live in `mod rmc_rt`, and the
// names are re-exported with `use`. rmc drops `mod` bodies and `use` imports
// and resolves these names as builtins; rustc compiles the module and imports
// them. Both see the same calls, which is what lets one file drive all four.
mod rmc_rt {
    extern "C" {
        pub fn print_i64(x: i64);
        pub fn arg_i64(i: i64) -> i64;
        pub fn alloc(n: i64) -> i64;
        pub fn free(p: i64);
        pub fn putchar(c: i64);
        pub fn exit(code: i64) -> !;
    }
    use core::panic::PanicInfo;
    #[panic_handler]
    pub fn panic(_info: &PanicInfo) -> ! {
        unsafe { exit(101) }
    }
}
use rmc_rt::print_i64;
use rmc_rt::arg_i64;
use rmc_rt::alloc;
use rmc_rt::free;
use rmc_rt::putchar;

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

fn emit(x: i64) {
    print(x)
}

// Backtracking search: N-queens with bitmasks, the shape of a constraint
// solver (sudoku, scheduling, SAT-style search).
fn solve(n: i64, row: i64, cols: i64, d1: i64, d2: i64) -> i64 {
    if row == n {
        return 1;
    }
    let mut count: i64 = 0;
    let full = (1 << n) - 1;
    let mut avail = !(cols | d1 | d2) & full;
    while avail != 0 {
        let bit = avail & (0 - avail);
        avail ^= bit;
        count += solve(n, row + 1, cols | bit, ((d1 | bit) << 1) & full, (d2 | bit) >> 1);
    }
    count
}

fn run(n: i64) {
    let n = if n == 0 { 13 } else { n };
    emit(solve(n, 0, 0, 0, 0));
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
