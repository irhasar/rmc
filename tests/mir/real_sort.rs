#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (sort), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
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
    out(check);
    out(bad);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
