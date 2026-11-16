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

// Sorting: median-of-three quicksort with an insertion-sort cutoff, the shape
// of a hand-written sort in a data-processing tool. Repeated over fresh data.
const N: usize = 200000;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn insertion(a: &mut [i64; N], lo: i64, hi: i64) {
    let mut i = lo + 1;
    while i <= hi {
        let v = a[i as usize];
        let mut j = i - 1;
        while j >= lo && a[j as usize] > v {
            a[(j + 1) as usize] = a[j as usize];
            j -= 1;
        }
        a[(j + 1) as usize] = v;
        i += 1;
    }
}

fn quicksort(a: &mut [i64; N], lo: i64, hi: i64) {
    if hi - lo < 16 {
        insertion(a, lo, hi);
        return;
    }
    let mid = lo + (hi - lo) / 2;
    let x = a[lo as usize];
    let y = a[mid as usize];
    let z = a[hi as usize];
    let pivot = if x < y {
        if y < z { y } else if x < z { z } else { x }
    } else {
        if x < z { x } else if y < z { z } else { y }
    };
    let mut i = lo;
    let mut j = hi;
    while i <= j {
        while a[i as usize] < pivot {
            i += 1;
        }
        while a[j as usize] > pivot {
            j -= 1;
        }
        if i <= j {
            let t = a[i as usize];
            a[i as usize] = a[j as usize];
            a[j as usize] = t;
            i += 1;
            j -= 1;
        }
    }
    if lo < j {
        quicksort(a, lo, j);
    }
    if i < hi {
        quicksort(a, i, hi);
    }
}

fn run(n: i64) {
    let rounds = if n == 0 { 20 } else { n };
    let mut a: [i64; N] = [0; N];
    let mut seed: i64 = 88172645463325252;
    let mut check: i64 = 0;
    let mut bad: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        let mut i: usize = 0;
        while i < N {
            seed = xorshift(seed);
            a[i] = (seed >> 1) % 1000000007;
            i += 1;
        }
        quicksort(&mut a, 0, (N as i64) - 1);
        let mut k: usize = 1;
        while k < N {
            if a[k - 1] > a[k] {
                bad += 1;
            }
            k += 1;
        }
        check = check.wrapping_mul(31).wrapping_add(a[N / 2]).wrapping_add(a[N - 1]);
        r += 1;
    }
    emit(check);
    emit(bad);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
