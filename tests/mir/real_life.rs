#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (life), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Cellular automaton: Conway's Game of Life on a 128x128 torus, the shape of
// a grid simulation (image filters, physics stencils, board games).
const W: i64 = 128;
const CELLS: usize = 16384;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn run(n: i64) {
    let gens = if n == 0 { 1500 } else { n };
    let mut cur: [i64; CELLS] = [0; CELLS];
    let mut nxt: [i64; CELLS] = [0; CELLS];
    let mut seed: i64 = 99991;
    let mut c: usize = 0;
    while c < CELLS {
        seed = xorshift(seed);
        cur[c] = if (seed >> 1) % 3 == 0 { 1 } else { 0 };
        c += 1;
    }
    let mut g: i64 = 0;
    while g < gens {
        let mut y: i64 = 0;
        while y < W {
            let up = ((y + W - 1) & (W - 1)) * W;
            let mid = y * W;
            let dn = ((y + 1) & (W - 1)) * W;
            let mut x: i64 = 0;
            while x < W {
                let l = (x + W - 1) & (W - 1);
                let r = (x + 1) & (W - 1);
                let s = cur[(up + l) as usize] + cur[(up + x) as usize] + cur[(up + r) as usize]
                    + cur[(mid + l) as usize] + cur[(mid + r) as usize]
                    + cur[(dn + l) as usize] + cur[(dn + x) as usize] + cur[(dn + r) as usize];
                let alive = cur[(mid + x) as usize];
                nxt[(mid + x) as usize] = if s == 3 || (s == 2 && alive == 1) { 1 } else { 0 };
                x += 1;
            }
            y += 1;
        }
        let mut k: usize = 0;
        while k < CELLS {
            cur[k] = nxt[k];
            k += 1;
        }
        g += 1;
    }
    let mut live: i64 = 0;
    let mut k: usize = 0;
    while k < CELLS {
        live += cur[k];
        k += 1;
    }
    out(live);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
