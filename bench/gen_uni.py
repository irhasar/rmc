#!/usr/bin/env python3
"""Generate bench/uni/*.rs: one source per program that all four pipelines
accept (rmc's AST front end, and rustc driving rmc / Cranelift / LLVM).

The shared header is the whole trick. It puts the freestanding runtime
declarations and the panic handler inside `mod rmc_rt`, then re-exports the
names with `use`. rmc drops `mod` bodies and `use` imports and resolves those
names as its own builtins; rustc compiles the module and the imports. Both
therefore see the same calls, so the same file drives every pipeline.
"""
import os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)))

HEADER = '''#![no_std]
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
'''

BODIES = {
    "hello": '''
#[no_mangle]
pub fn main() -> i64 {
    print(42);
    0
}
''',
    "fib": '''
#[inline(never)]
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n;
    }
    fib(n - 1) + fib(n - 2)
}

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 32;
    }
    print(fib(n));
    0
}
''',
    "mandel": '''
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
''',
    "sieve": '''
const N: usize = 200000;

#[no_mangle]
pub fn main() -> i64 {
    let mut limit: i64 = arg(1);
    if limit == 0 {
        limit = 200000;
    }
    let mut flags: [i64; N] = [1; N];
    let mut count: i64 = 0;
    let mut i: i64 = 2;
    while i < limit {
        if flags[i as usize] == 1 {
            count = count + 1;
            let mut j: i64 = i * i;
            while j < limit {
                flags[j as usize] = 0;
                j = j + i;
            }
        }
        i = i + 1;
    }
    print(count);
    0
}
''',
    "mix": '''
#[inline(never)]
fn gcd(a: i64, b: i64) -> i64 {
    let mut x: i64 = a;
    let mut y: i64 = b;
    while y != 0 {
        let t: i64 = x % y;
        x = y;
        y = t;
    }
    x
}

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut sum: i64 = 0;
    let mut i: i64 = 1;
    while i < n {
        sum = sum + gcd(i, 2 * i + 3);
        i = i + 1;
    }
    print(sum);
    0
}
''',
    "stats": '''
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
''',
    "refs": '''
struct Wide {
    a: i64,
    b: i64,
    c: i64,
    d: i64,
    e: i64,
    f: i64,
    g: i64,
    h: i64,
}

impl Wide {
    #[inline(never)]
    fn total(&self) -> i64 {
        self.a + self.b + self.c + self.d + self.e + self.f + self.g + self.h
    }
    #[inline(never)]
    fn pair(&self) -> i64 {
        self.a * self.h + self.d * self.e
    }
}

#[inline(never)]
fn sum_via_ref(w: &Wide) -> i64 {
    w.total() + w.pair()
}

#[no_mangle]
pub fn main() -> i64 {
    let n: i64 = arg(1);
    let w: Wide = Wide { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8 };
    let wr: &Wide = &w;
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        acc = acc + wr.total() + sum_via_ref(wr) + wr.pair();
        i = i + 1;
    }
    print(acc);
    0
}
''',
    "heap": '''
struct Node {
    val: i64,
    next: i64,
}

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 200000;
    }
    let mut head: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let p: i64 = alc(16);
        let np: &mut Node = unsafe { &mut *(p as *mut Node) };
        np.val = i;
        np.next = head;
        head = p;
        i = i + 1;
    }
    let mut sum: i64 = 0;
    let mut cur: i64 = head;
    while cur != 0 {
        let cp: &Node = unsafe { &*(cur as *const Node) };
        sum = sum + cp.val;
        cur = cp.next;
    }
    print(sum);
    0
}
''',
    "callrec": '''
#[inline(never)]
fn tree(n: i64) -> i64 {
    if n <= 1 {
        return n;
    }
    1 + tree(n - 1) + tree(n - 2)
}

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 30;
    }
    print(tree(n));
    0
}
''',
    "forloops": '''
struct Acc {
    sum: i64,
    cnt: i64,
}

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut a: Acc = Acc { sum: 0, cnt: 0 };
    let mut i: i64 = 0;
    while i < n {
        a.sum = a.sum + i;
        if a.sum > 1000000000 {
            a.sum = a.sum - 1000000000;
            a.cnt = a.cnt + 1;
        }
        i = i + 1;
    }
    i = 0;
    while i <= 1000 {
        let mut j: i64 = 0;
        while j <= 10 {
            a.sum = a.sum + (i ^ j);
            j = j + 1;
        }
        i = i + 1;
    }
    print(a.sum);
    print(a.cnt);
    0
}
''',
    "light": '''
const LEN: usize = 256;

#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 2000000;
    }
    let mut table: [i64; LEN] = [0; LEN];
    let mut i: i64 = 0;
    while i < LEN as i64 {
        table[i as usize] = (i.wrapping_mul(2654435761)) % 1000003;
        i = i + 1;
    }
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let v: i64 = table[(i & 255) as usize];
        acc = acc + v;
        if acc > 1000000 {
            acc = acc - 1000000;
        }
        i = i + 1;
    }
    print(acc);
    0
}
''',
    "heavy": '''
#[no_mangle]
pub fn main() -> i64 {
    let mut n: i64 = arg(1);
    if n == 0 {
        n = 1000000000;
    }
    let mut sum: i64 = 0;
    let mut alt: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        sum = sum + i;
        if sum > 1000000000 {
            sum = sum - 1000000000;
        }
        alt = alt + (i & 1);
        i = i + 1;
    }
    print(sum);
    print(alt);
    0
}
''',
}


def main():
    d = os.path.join(OUT, "uni")
    os.makedirs(d, exist_ok=True)
    for name, body in BODIES.items():
        path = os.path.join(d, f"{name}.rs")
        with open(path, "w") as f:
            f.write(HEADER + body)
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
