#![no_std]
#![no_main]
include!("prelude.rs");

use core::hint::black_box;

// More values are live through each inner loop than there are registers, so
// the back end saves the ones the loop never mentions before it and restores
// them at every exit (`split_loop_ranges`). The exits here are the ordinary
// one, a `break` to the same place, an early `return` out of two loops, and a
// loop whose entry block ends in a call; each restored value is then read
// and changed again, so a missed save or restore shows in the output.

#[inline(never)]
fn id(x: i64) -> i64 {
    black_box(x)
}

#[inline(never)]
fn nest(n: i64, k: i64) -> i64 {
    // Ten accumulators the inner loops do not touch.
    let (mut a, mut b, mut c, mut d, mut e) = (k, k + 1, k + 2, k + 3, k + 4);
    let (mut f, mut g, mut h, mut p, mut q) = (k * 2, k * 3, k * 5, k * 7, k * 11);
    let mut i: i64 = 0;
    'outer: while i < n {
        // Inner loop 1: its own temporaries only, with a `break`, and a
        // `break 'outer` whose target the outer loop's own exit shares.
        let mut s: i64 = 0;
        let mut j: i64 = 0;
        while j < 20 {
            let t = (j * 7 + i) ^ (s >> 1);
            let u = t & 1023;
            s += u * 3 + (t >> 4);
            if s > 100000 + k {
                break;
            }
            if j == 3 && i == k + 2 {
                break 'outer;
            }
            j += 1;
        }
        a += s;
        b ^= a + j;
        // Inner loop 2: entered right after a call whose result (`h`) the
        // loop does not mention, and containing a call.
        let mut w = i + c;
        h = id(h + 1);
        let mut m: i64 = 0;
        while m < 3 {
            w = id(w + m) + (w >> 2);
            if w == k * 1000 {
                // An exit out of both loops.
                return a + b + c + d + e + f + g + h + p + q + w;
            }
            m += 1;
        }
        c += w & 255;
        d = d * 3 + c;
        e += d ^ i;
        f -= e & 15;
        g += f + a;
        h ^= g;
        p += h & 7;
        q = q * 5 + p;
        i += 1;
    }
    a + b + c + d + e + f + g + h + p + q
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(1);
    let k = arg(2);
    out(nest(n, k));
    out(nest(n + 3, -k));
    0
}
