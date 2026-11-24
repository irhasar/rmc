#![no_std]
#![no_main]
include!("prelude.rs");

// A call's arguments are moved into `%rdi`..`%r9` as one parallel
// assignment. Each function below passes its own incoming arguments on in a
// permuted order while they still sit in their incoming registers (they die at
// the call), so the moves form cycles: a swap, a rotation of three, and two
// independent swaps. The non-tail `^ n` keeps rustc and rmc from turning the
// recursion into a loop.

#[inline(never)]
fn swap(a: i64, b: i64, n: i64) -> i64 {
    if n == 0 {
        return a * 1000 + b;
    }
    swap(b, a + 1, n - 1) ^ n
}

#[inline(never)]
fn rotate(a: i64, b: i64, c: i64, n: i64) -> i64 {
    if n == 0 {
        return a * 10000 + b * 100 + c;
    }
    rotate(c, a, b, n - 1) ^ (n * 3)
}

#[inline(never)]
fn two_swaps(a: i64, b: i64, c: i64, d: i64, n: i64) -> i64 {
    if n == 0 {
        return ((a * 31 + b) * 31 + c) * 31 + d;
    }
    two_swaps(b, a, d, c - 1, n - 1) ^ (n << 2)
}

// A rotation through all six argument registers, plus one argument read
// through a reference while its base register is being reassigned.
#[inline(never)]
fn six(a: i64, b: i64, c: i64, d: i64, e: i64, n: &[i64; 2]) -> i64 {
    if n[0] == 0 {
        return a - 2 * b + 3 * c - 4 * d + 5 * e + n[1];
    }
    let next = [n[0] - 1, n[1] + a];
    six(e, a, b, c, d, &next) ^ n[0]
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let m = arg(2);
    out(swap(k, m, 5));
    out(swap(m, k, 6));
    out(rotate(k, m, k + m, 7));
    out(rotate(1, 2, 3, k & 15));
    out(two_swaps(k, m, k - m, 9, 6));
    out(six(k, m, 3, 4, 5, &[k & 7, m]));
    0
}
