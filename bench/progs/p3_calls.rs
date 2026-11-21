// function calls, recursion, string ops
fn fib(n: u64) -> u64 { if n < 2 { n } else { fib(n-1) + fib(n-2) } }
#[inline(never)]
fn mix(s: &str) -> usize { s.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)) }
fn main() {
    let mut h = 0usize;
    for i in 0..30u64 { h ^= fib(27 + (i % 3)) as usize; }
    for i in 0..200_000 { h = h.wrapping_add(mix(&format!("item-{}-xyz", i))); }
    println!("{}", h);
}
