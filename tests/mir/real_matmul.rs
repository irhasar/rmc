#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (matmul), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Dense integer matrix multiply (128x128), the shape of a numeric kernel
// (image transforms, graph adjacency powers, fixed-point ML inference).
const N: i64 = 128;
const NN: usize = 16384;

fn matmul(a: &[i64; NN], b: &[i64; NN], c: &mut [i64; NN]) {
    let mut i: i64 = 0;
    while i < N {
        let mut j: i64 = 0;
        while j < N {
            let mut s: i64 = 0;
            let mut k: i64 = 0;
            while k < N {
                s += a[(i * N + k) as usize] * b[(k * N + j) as usize];
                k += 1;
            }
            c[(i * N + j) as usize] = s;
            j += 1;
        }
        i += 1;
    }
}

fn run(n: i64) {
    let rounds = if n == 0 { 40 } else { n };
    let mut a: [i64; NN] = [0; NN];
    let mut b: [i64; NN] = [0; NN];
    let mut c: [i64; NN] = [0; NN];
    let mut i: usize = 0;
    while i < NN {
        a[i] = ((i as i64) * 7 + 3) % 1000;
        b[i] = ((i as i64) * 13 + 5) % 1000;
        i += 1;
    }
    let mut check: i64 = 0;
    let mut r: i64 = 0;
    while r < rounds {
        matmul(&a, &b, &mut c);
        let mut k: usize = 0;
        while k < NN {
            a[k] = c[k] % 1000;
            k += 1;
        }
        check = check.wrapping_mul(31).wrapping_add(c[(r as usize * 97) % NN]);
        r += 1;
    }
    out(check);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
