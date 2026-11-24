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

// Cellular automaton: Conway's Game of Life on a 128x128 torus, the shape of
// a grid simulation (image filters, physics stencils, board games).
const W: i64 = 128;
const CELLS: usize = 16384;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn run(n: i64) {
    let gens = if n == 0 { 1500 } else { n };
    let mut cur: [i64; CELLS] = [0; CELLS];
    let mut nxt: [i64; CELLS] = [0; CELLS];
    let mut seed: i64 = 99991;
    let mut c: usize = 0;
    while c < CELLS {
        seed = xorshift(seed);
        cur[c] = if (seed >> 1) % 3 == 0 { 1 } else { 0 };
        c += 1;
    }
    let mut g: i64 = 0;
    while g < gens {
        let mut y: i64 = 0;
        while y < W {
            let up = ((y + W - 1) & (W - 1)) * W;
            let mid = y * W;
            let dn = ((y + 1) & (W - 1)) * W;
            let mut x: i64 = 0;
            while x < W {
                let l = (x + W - 1) & (W - 1);
                let r = (x + 1) & (W - 1);
                let s = cur[(up + l) as usize] + cur[(up + x) as usize] + cur[(up + r) as usize]
                    + cur[(mid + l) as usize] + cur[(mid + r) as usize]
                    + cur[(dn + l) as usize] + cur[(dn + x) as usize] + cur[(dn + r) as usize];
                let alive = cur[(mid + x) as usize];
                nxt[(mid + x) as usize] = if s == 3 || (s == 2 && alive == 1) { 1 } else { 0 };
                x += 1;
            }
            y += 1;
        }
        let mut k: usize = 0;
        while k < CELLS {
            cur[k] = nxt[k];
            k += 1;
        }
        g += 1;
    }
    let mut live: i64 = 0;
    let mut k: usize = 0;
    while k < CELLS {
        live += cur[k];
        k += 1;
    }
    emit(live);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
