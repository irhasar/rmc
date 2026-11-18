fn main() {
    // affine counted loop with carried u64 state (unroll target)
    let mut acc: u64 = 0;
    for i in 0..1_000_000u64 { acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i); }
    // nested loops
    let mut total = 0u64;
    for i in 0..1000u64 { for j in 0..1000u64 { total = total.wrapping_add(i.wrapping_mul(j) ^ (i+j)); } }
    // while loop with memory
    let mut v: Vec<u64> = vec![0; 1000];
    let mut k = 0usize;
    while k < 1000 { v[k] = (k as u64).wrapping_mul(2654435761); k += 1; }
    let s: u64 = v.iter().copied().fold(0, u64::wrapping_add);
    // loop with break/continue
    let mut b = 0u64; let mut c = 0u64;
    for i in 0..10000u64 { if i % 3 == 0 { continue; } if i > 8000 { break; } b = b.wrapping_add(i); c ^= b; }
    println!("{} {} {} {}", acc, total, s, c);
}
