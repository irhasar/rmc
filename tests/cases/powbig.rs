// A `const` `pow` with a huge literal exponent folds in O(log exponent) by
// squaring, not O(exponent) multiplications: `1u8.pow(1_000_000_000)` is a
// valid program that must compile in milliseconds, not stall for seconds. The
// base is squared only while exponent bits remain, so an unused high square
// never rejects a result that fits. `0^anything` and `x^0` pin the edges.
const ONE: u8 = 1u8.pow(1_000_000_000);
const ZERO: u8 = 0u8.pow(1_000_000_000);
const EXP0: u8 = 3u8.pow(0);

fn main() {
    println!("{} {} {}", ONE, ZERO, EXP0);
}
