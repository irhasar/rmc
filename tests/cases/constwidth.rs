// Constants fold at their declared width and through the arithmetic methods
// rustc folds in a `const`: a forward reference resolves, `wrapping_*` wraps,
// `saturating_*` clamps and `pow` is exact within the width. Every value stays
// in range, so this runs without overflow checks and still pins the width.

const A: u8 = B;
const B: u8 = 7;

const W: u8 = 200u8.wrapping_mul(2);
const WA: u8 = 250u8.wrapping_add(10);
const WS: u8 = 3u8.wrapping_sub(10);
const S: u8 = 250u8.saturating_add(10);
const SM: i8 = 100i8.saturating_mul(2);
const P: u8 = 3u8.pow(5);

const SUM: u8 = 20 * 10;
const NARROW: u8 = 5;

fn main() {
    println!("{}", A);
    println!("{}", W);
    println!("{}", WA);
    println!("{}", WS);
    println!("{}", S);
    println!("{}", SM);
    println!("{}", P);
    println!("{}", SUM);

    // A narrow constant used in a wider expression adopts the wider width.
    println!("{}", NARROW + 200);
}
