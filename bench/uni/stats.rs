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

struct Acc {
    sum: i64,
    count: i64,
    max: i64,
    min: i64,
}

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut a: Acc = Acc {
        sum: 0,
        count: 0,
        max: -9223372036854775807,
        min: 9223372036854775807,
    };
    let mut i: i64 = 1;
    while i < n {
        let v: i64 = (i * 2654435761) % 1000003;
        a.sum = a.sum + v;
        a.count = a.count + 1;
        if v > a.max {
            a.max = v;
        }
        if v < a.min {
            a.min = v;
        }
        i = i + 1;
    }
    print(a.sum);
    print(a.max);
    print(a.min);
    print(a.count);
    0
}
