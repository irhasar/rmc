#![no_std]
#![no_main]
include!("prelude.rs");

// Locals whose lifetimes never overlap may share a register, so every read
// must be seen by the liveness analysis, including the reads that are not in
// an rvalue: the callee of an indirect call, the index of a store
// destination (`a[i] = t`), and the operands of a comparison that is folded
// into the branch that tests it. A missed read lets a later temporary take
// the register and overwrite the value before that read.

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn sub(a: i64, b: i64) -> i64 {
    a - b
}

// The callee is chosen first, then several temporaries are computed before
// the call reads it.
#[inline(never)]
fn indirect(k: i64, x: i64, y: i64) -> i64 {
    let f: fn(i64, i64) -> i64 = if k > 0 { add } else { sub };
    let p = x * 7 + y;
    let q = p ^ (x << 3);
    let r = q - (y >> 1);
    f(r, p) + f(q, 1)
}

// The index is computed before the value stored through it.
#[inline(never)]
fn store_index(k: i64, i: usize) -> i64 {
    let mut a: [i64; 16] = [0; 16];
    let mut n: usize = 0;
    while n < 6 {
        let j = (i + n * 3) % 16;
        let t = k * (n as i64) + 5;
        let u = t ^ (k >> 2);
        a[j] = u - 1;
        n += 1;
    }
    let mut s: i64 = 0;
    let mut m: usize = 0;
    while m < 16 {
        s = s * 3 + a[m];
        m += 1;
    }
    s
}

// Comparison operands stay live up to the branch the comparison folds into.
#[inline(never)]
fn folded(k: i64, x: i64) -> i64 {
    let mut s: i64 = 0;
    let mut i: i64 = 0;
    while i < k {
        let a = x + i;
        let b = (i * 5) & 15;
        let c = a * 3 - b;
        if c < a + b {
            s += c;
        } else {
            s -= b;
        }
        i += 1;
    }
    s
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let x = arg(2);
    out(indirect(k, x, k - x));
    out(indirect(-k, x, x + 3));
    out(store_index(k, x as usize));
    out(folded(k, x));
    out(folded(k + 7, -x));
    0
}
