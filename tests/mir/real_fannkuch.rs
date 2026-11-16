#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (fannkuch), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// fannkuch-redux (Computer Language Benchmarks Game), sequential version:
// permutation generation and prefix reversal over small arrays.
fn fannkuch(n: i64, res: &mut [i64; 2]) {
    let mut perm: [i64; 16] = [0; 16];
    let mut perm1: [i64; 16] = [0; 16];
    let mut count: [i64; 16] = [0; 16];
    let mut i: i64 = 0;
    while i < n {
        perm1[i as usize] = i;
        i += 1;
    }
    let mut checksum: i64 = 0;
    let mut maxflips: i64 = 0;
    let mut perm_count: i64 = 0;
    let mut r: i64 = n;
    'outer: loop {
        while r != 1 {
            count[(r - 1) as usize] = r;
            r -= 1;
        }
        let mut k: i64 = 0;
        while k < n {
            perm[k as usize] = perm1[k as usize];
            k += 1;
        }
        let mut flips: i64 = 0;
        let mut first = perm[0];
        while first != 0 {
            let mut lo: i64 = 0;
            let mut hi: i64 = first;
            while lo < hi {
                let t = perm[lo as usize];
                perm[lo as usize] = perm[hi as usize];
                perm[hi as usize] = t;
                lo += 1;
                hi -= 1;
            }
            flips += 1;
            first = perm[0];
        }
        if flips > maxflips {
            maxflips = flips;
        }
        if perm_count % 2 == 0 {
            checksum += flips;
        } else {
            checksum -= flips;
        }
        loop {
            if r == n {
                break 'outer;
            }
            let perm0 = perm1[0];
            let mut j: i64 = 0;
            while j < r {
                perm1[j as usize] = perm1[(j + 1) as usize];
                j += 1;
            }
            perm1[r as usize] = perm0;
            count[r as usize] -= 1;
            if count[r as usize] > 0 {
                break;
            }
            r += 1;
        }
        perm_count += 1;
    }
    res[0] = checksum;
    res[1] = maxflips;
}

fn run(n: i64) {
    let n = if n == 0 { 10 } else { n };
    let mut res: [i64; 2] = [0; 2];
    fannkuch(n, &mut res);
    out(res[0]);
    out(res[1]);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
