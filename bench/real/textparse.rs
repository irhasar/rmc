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

// Text processing: generate a CSV-like byte buffer of decimal numbers, then
// scan and parse it repeatedly, the shape of a log/CSV parser.
const CAP: usize = 1048576;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn generate(buf: &mut [u8; CAP], seed0: i64) -> i64 {
    let mut seed = seed0;
    let mut len: i64 = 0;
    let mut field: i64 = 0;
    let mut tmp: [u8; 24] = [0; 24];
    while len < (CAP as i64) - 32 {
        seed = xorshift(seed);
        let mut v = (seed >> 1) & 1048575;
        let mut t: i64 = 0;
        if v == 0 {
            tmp[0] = b'0';
            t = 1;
        }
        while v > 0 {
            tmp[t as usize] = b'0' + (v % 10) as u8;
            v /= 10;
            t += 1;
        }
        while t > 0 {
            t -= 1;
            buf[len as usize] = tmp[t as usize];
            len += 1;
        }
        field += 1;
        if field % 10 == 0 {
            buf[len as usize] = b'\n';
        } else {
            buf[len as usize] = b',';
        }
        len += 1;
    }
    len
}

fn parse(buf: &[u8; CAP], len: i64) -> i64 {
    let mut i: i64 = 0;
    let mut cur: i64 = 0;
    let mut sum: i64 = 0;
    let mut lines: i64 = 0;
    let mut maxv: i64 = 0;
    while i < len {
        let c = buf[i as usize];
        if c >= b'0' && c <= b'9' {
            cur = cur * 10 + (c - b'0') as i64;
        } else {
            sum += cur;
            if cur > maxv {
                maxv = cur;
            }
            cur = 0;
            if c == b'\n' {
                lines += 1;
            }
        }
        i += 1;
    }
    sum + lines * 1000003 + maxv
}

fn run(n: i64) {
    let rounds = if n == 0 { 300 } else { n };
    let mut buf: [u8; CAP] = [0; CAP];
    let len = generate(&mut buf, 777777);
    let mut check: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        check = check.wrapping_mul(7).wrapping_add(parse(&buf, len - (r % 7)));
        r += 1;
    }
    emit(len);
    emit(check);
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
