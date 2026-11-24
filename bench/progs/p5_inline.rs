// Small hot function that a good inliner folds away; measures cross-function call overhead.
#[inline(always)]
fn add3(a: u64, b: u64) -> u64 { a.wrapping_add(b).wrapping_mul(3) }
fn main() {
    let mut acc = 0u64;
    for i in 0..300_000_000u64 { acc = add3(acc, i); }
    println!("{}", acc);
}
