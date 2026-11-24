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

// Physics simulation: fixed-point particles with gravity and wall bounces,
// an array of structs updated in place (game loop, particle system).
#[derive(Clone, Copy)]
struct P {
    x: i64,
    y: i64,
    vx: i64,
    vy: i64,
}

const NP: usize = 2000;
const LIMIT: i64 = 1000000;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn step(ps: &mut [P; NP]) {
    let mut i: usize = 0;
    while i < NP {
        let mut p = ps[i];
        p.vy -= 3;
        p.x += p.vx;
        p.y += p.vy;
        if p.x < 0 {
            p.x = -p.x;
            p.vx = -p.vx;
        }
        if p.x > LIMIT {
            p.x = 2 * LIMIT - p.x;
            p.vx = -p.vx;
        }
        if p.y < 0 {
            p.y = -p.y;
            p.vy = -p.vy * 9 / 10;
        }
        if p.y > LIMIT {
            p.y = 2 * LIMIT - p.y;
            p.vy = -p.vy;
        }
        ps[i] = p;
        i += 1;
    }
}

fn run(n: i64) {
    let steps = if n == 0 { 20000 } else { n };
    let mut ps: [P; NP] = [P { x: 0, y: 0, vx: 0, vy: 0 }; NP];
    let mut seed: i64 = 4242;
    let mut i: usize = 0;
    while i < NP {
        seed = xorshift(seed);
        let a = (seed >> 1) % LIMIT;
        seed = xorshift(seed);
        let b = (seed >> 1) % LIMIT;
        seed = xorshift(seed);
        let c = (seed >> 1) % 2001 - 1000;
        seed = xorshift(seed);
        let d = (seed >> 1) % 2001 - 1000;
        ps[i] = P { x: a, y: b, vx: c, vy: d };
        i += 1;
    }
    let mut s: i64 = 0;
    while s < steps {
        step(&mut ps);
        s += 1;
    }
    let mut energy: i64 = 0;
    let mut cx: i64 = 0;
    let mut k: usize = 0;
    while k < NP {
        energy += ps[k].vx * ps[k].vx + ps[k].vy * ps[k].vy;
        cx += ps[k].x;
        k += 1;
    }
    emit(energy);
    emit(cx / (NP as i64));
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
