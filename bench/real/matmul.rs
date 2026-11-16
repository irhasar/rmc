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

// Dense integer matrix multiply (128x128), the shape of a numeric kernel
// (image transforms, graph adjacency powers, fixed-point ML inference).
const N: i64 = 128;
const NN: usize = 16384;

fn matmul(a: &[i64; NN], b: &[i64; NN], c: &mut [i64; NN]) {
    let mut i: i64 = 0;
    while i < N {
        let mut j: i64 = 0;
        while j < N {
            let mut s: i64 = 0;
            let mut k: i64 = 0;
            while k < N {
                s += a[(i * N + k) as usize] * b[(k * N + j) as usize];
                k += 1;
            }
            c[(i * N + j) as usize] = s;
            j += 1;
        }
        i += 1;
    }
}

fn run(n: i64) {
    let rounds = if n == 0 { 40 } else { n };
    let mut a: [i64; NN] = [0; NN];
    let mut b: [i64; NN] = [0; NN];
    let mut c: [i64; NN] = [0; NN];
    let mut i: usize = 0;
    while i < NN {
        a[i] = ((i as i64) * 7 + 3) % 1000;
        b[i] = ((i as i64) * 13 + 5) % 1000;
        i += 1;
    }
    let mut check: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        matmul(&a, &b, &mut c);
        let mut k: usize = 0;
        while k < NN {
            a[k] = c[k] % 1000;
            k += 1;
        }
        check = check.wrapping_mul(31).wrapping_add(c[(r as usize * 97) % NN]);
        r += 1;
    }
    emit(check);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
