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

// fannkuch-redux (Computer Language Benchmarks Game), sequential version:
// permutation generation and prefix reversal over small arrays.
fn fannkuch(n: i64, res: &mut [i64; 2]) {
    let mut perm: [i64; 16] = [0; 16];
    let mut perm1: [i64; 16] = [0; 16];
    let mut count: [i64; 16] = [0; 16];
    let mut i: i64 = 0;
    while i < n {
        perm1[i as usize] = i;
        i += 1;
    }
    let mut checksum: i64 = 0;
    let mut maxflips: i64 = 0;
    let mut perm_count: i64 = 0;
    let mut r: i64 = n;
    'outer: loop {
        while r != 1 {
            count[(r - 1) as usize] = r;
            r -= 1;
        }
        let mut k: i64 = 0;
        while k < n {
            perm[k as usize] = perm1[k as usize];
            k += 1;
        }
        let mut flips: i64 = 0;
        let mut first = perm[0];
        while first != 0 {
            let mut lo: i64 = 0;
            let mut hi: i64 = first;
            while lo < hi {
                let t = perm[lo as usize];
                perm[lo as usize] = perm[hi as usize];
                perm[hi as usize] = t;
                lo += 1;
                hi -= 1;
            }
            flips += 1;
            first = perm[0];
        }
        if flips > maxflips {
            maxflips = flips;
        }
        if perm_count % 2 == 0 {
            checksum += flips;
        } else {
            checksum -= flips;
        }
        loop {
            if r == n {
                break 'outer;
            }
            let perm0 = perm1[0];
            let mut j: i64 = 0;
            while j < r {
                perm1[j as usize] = perm1[(j + 1) as usize];
                j += 1;
            }
            perm1[r as usize] = perm0;
            count[r as usize] -= 1;
            if count[r as usize] > 0 {
                break;
            }
            r += 1;
        }
        perm_count += 1;
    }
    res[0] = checksum;
    res[1] = maxflips;
}

fn run(n: i64) {
    let n = if n == 0 { 10 } else { n };
    let mut res: [i64; 2] = [0; 2];
    fannkuch(n, &mut res);
    emit(res[0]);
    emit(res[1]);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
