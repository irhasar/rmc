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

// Hash table: open addressing with linear probing, counting key frequencies
// and then answering lookups. The shape of a word-count / dedup / cache index.
const CAP: usize = 131072;
const MASK: i64 = 131071;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn hash(k: i64) -> i64 {
    // Fibonacci hashing (0x9E3779B97F4A7C15 as i64).
    (k.wrapping_mul(-7046029254386353131) >> 20) & MASK
}

fn run(n: i64) {
    let ops = if n == 0 { 4000000 } else { n };
    let mut keys: [i64; CAP] = [-1; CAP];
    let mut vals: [i64; CAP] = [0; CAP];
    let mut seed: i64 = 2463534242;
    let mut distinct: i64 = 0;
    let mut i: i64 = 0;
    while i < ops {
        seed = xorshift(seed);
        let k = (seed >> 1) % 60000;
        let mut h = hash(k);
        loop {
            let kk = keys[h as usize];
            if kk == k {
                vals[h as usize] += 1;
                break;
            }
            if kk == -1 {
                keys[h as usize] = k;
                vals[h as usize] = 1;
                distinct += 1;
                break;
            }
            h = (h + 1) & MASK;
        }
        i += 1;
    }
    let mut hits: i64 = 0;
    let mut total: i64 = 0;
    let mut j: i64 = 0;
    while j < ops {
        seed = xorshift(seed);
        let k = (seed >> 1) % 90000;
        let mut h = hash(k);
        loop {
            let kk = keys[h as usize];
            if kk == k {
                hits += 1;
                total += vals[h as usize];
                break;
            }
            if kk == -1 {
                break;
            }
            h = (h + 1) & MASK;
        }
        j += 1;
    }
    emit(distinct);
    emit(hits);
    emit(total);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
