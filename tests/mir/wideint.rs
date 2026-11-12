#![no_std]
#![no_main]
include!("prelude.rs");

// 128-bit integer arithmetic. `i128` is two registers, so a load, store, add,
// multiply or comparison that uses one word gives a wrong low or high half -
// which is exactly what a naive lowering does. This case is why the back end
// must not claim "128-bit integers are not implemented" (see
// `classify_unsupported`).
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a: i128 = 1;
    let b: i128 = 2;

    // A multiply that overflows 64 bits: the high word is wrong if only the low
    // half is computed. 2^40 * 2^40 = 2^80, whose low 64 bits are 0.
    let big: i128 = 1i128 << 40;
    let sq = big * big;
    out((sq == (1i128 << 80)) as i64);
    out((sq >> 64) as i64);           // 65536

    // Carry out of the low word into the high word.
    let m: i128 = (1i128 << 63) + (1i128 << 63);   // 2^64, needs the high word
    out((m >> 32) as i64);            // 4294967296
    out((m == (1i128 << 64)) as i64);

    // Division and remainder on a value wider than 64 bits.
    let n: i128 = (1i128 << 64) + 100;
    out((n / 7) as i64);
    out((n % 7) as i64);

    // Signed comparison across the 64-bit boundary, both ways.
    outb(a < b);
    outb(big > a);
    let neg: i128 = 0 - (1i128 << 70);
    outb(neg < 0);
    outb(neg < big);

    // Cast down and back: a wrong truncation shows as a wrong sign or width.
    out(big as i64);                  // 2^40 fits in i64: 1099511627776
    let x: i64 = 1234567;
    out((x as i128 + 1) as i64);
    0
}
