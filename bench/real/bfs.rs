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

// Graph search: breadth-first search over a random grid maze, the shape of
// path-finding in a game, a router or a dependency resolver.
const W: i64 = 300;
const H: i64 = 300;
const CELLS: usize = 90000;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn run(n: i64) {
    let rounds = if n == 0 { 60 } else { n };
    let mut wall: [i64; CELLS] = [0; CELLS];
    let mut dist: [i64; CELLS] = [0; CELLS];
    let mut queue: [i64; CELLS] = [0; CELLS];
    let mut seed: i64 = 1234567;
    let mut sum_all: i64 = 0;
    let mut reach_all: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        let mut c: usize = 0;
        while c < CELLS {
            seed = xorshift(seed);
            wall[c] = if (seed >> 1) % 100 < 28 { 1 } else { 0 };
            dist[c] = -1;
            c += 1;
        }
        wall[0] = 0;
        dist[0] = 0;
        queue[0] = 0;
        let mut head: usize = 0;
        let mut tail: usize = 1;
        while head < tail {
            let cur = queue[head];
            head += 1;
            let x = cur % W;
            let y = cur / W;
            let d = dist[cur as usize] + 1;
            if x > 0 {
                let nb = cur - 1;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
            if x < W - 1 {
                let nb = cur + 1;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
            if y > 0 {
                let nb = cur - W;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
            if y < H - 1 {
                let nb = cur + W;
                if wall[nb as usize] == 0 && dist[nb as usize] < 0 {
                    dist[nb as usize] = d;
                    queue[tail] = nb;
                    tail += 1;
                }
            }
        }
        let mut k: usize = 0;
        while k < CELLS {
            if dist[k] > 0 {
                sum_all += dist[k];
            }
            k += 1;
        }
        reach_all += tail as i64;
        r += 1;
    }
    emit(reach_all);
    emit(sum_all);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
