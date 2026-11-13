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

#[no_mangle]
pub fn main() -> i64 {
    let mut size: i64 = arg(1);
    if size == 0 {
        size = 800;
    }
    let mut acc: i64 = 0;
    let mut y: i64 = 0;
    while y < size {
        let mut x: i64 = 0;
        while x < size {
            let mut zr: i64 = 0;
            let mut zi: i64 = 0;
            let cr: i64 = x - 200;
            let ci: i64 = y - 200;
            let mut i: i64 = 0;
            while i < 50 {
                let zr2: i64 = (zr * zr) / 1000;
                let zi2: i64 = (zi * zi) / 1000;
                if zr2 + zi2 > 4000 {
                    break;
                }
                let nzi: i64 = (2 * zr * zi) / 1000 + ci;
                zr = zr2 - zi2 + cr;
                zi = nzi;
                i = i + 1;
            }
            acc = acc + i;
            x = x + 1;
        }
        y = y + 1;
    }
    print(acc);
    0
}
