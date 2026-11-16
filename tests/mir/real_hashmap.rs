#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (hashmap), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Hash table: open addressing with linear probing, counting key frequencies
// and then answering lookups. The shape of a word-count / dedup / cache index.
const CAP: usize = 131072;
const MASK: i64 = 131071;

fn xorshift(s: i64) -> i64 {
    let mut x = s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn hash(k: i64) -> i64 {
    // Fibonacci hashing (0x9E3779B97F4A7C15 as i64).
    (k.wrapping_mul(-7046029254386353131) >> 20) & MASK
}

fn run(n: i64) {
    let ops = if n == 0 { 4000000 } else { n };
    let mut keys: [i64; CAP] = [-1; CAP];
    let mut vals: [i64; CAP] = [0; CAP];
    let mut seed: i64 = 2463534242;
    let mut distinct: i64 = 0;
    let mut i: i64 = 0;
    while i < ops {
        seed = xorshift(seed);
        let k = (seed >> 1) % 60000;
        let mut h = hash(k);
        loop {
            let kk = keys[h as usize];
            if kk == k {
                vals[h as usize] += 1;
                break;
            }
            if kk == -1 {
                keys[h as usize] = k;
                vals[h as usize] = 1;
                distinct += 1;
                break;
            }
            h = (h + 1) & MASK;
        }
        i += 1;
    }
    let mut hits: i64 = 0;
    let mut total: i64 = 0;
    let mut j: i64 = 0;
    while j < ops {
        seed = xorshift(seed);
        let k = (seed >> 1) % 90000;
        let mut h = hash(k);
        loop {
            let kk = keys[h as usize];
            if kk == k {
                hits += 1;
                total += vals[h as usize];
                break;
            }
            if kk == -1 {
                break;
            }
            h = (h + 1) & MASK;
        }
        j += 1;
    }
    out(distinct);
    out(hits);
    out(total);
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
