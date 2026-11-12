// A `const` method folds at its *receiver's* declared width, not the
// destination's: `A.saturating_add(10)` on a `u8` `A` clamps at `u8` (255) even
// when the result is bound to a wider type. The width comes from the receiver
// (a narrow constant name, a suffixed literal or a cast), falling back to the
// declared width only when the receiver names none. Every value is chosen to
// stay in range, so no overflow check fires and the case diffs against rustc.
const A: u8 = 250;
const SAT: i64 = A.saturating_add(10) as i64;
const WRAP: i64 = A.wrapping_add(10) as i64;

const U16: u16 = 3;
const P: u16 = U16.pow(5);

fn main() {
    println!("{} {}", SAT, WRAP);
    println!("{}", P);
}
