// Regression coverage for the iterator-chain inlining budget.
//
// `collect` on a `map` over a range lowers to `extend_trusted`, which hands the
// element loop to `fold`. With the old caps that loop kept two real calls per
// element (`Range::spec_next` and the `map_fold` closure) plus the spill and
// reload of the iterator state around each one, which made the whole collect
// ten times slower than the LLVM build. The caps are now large enough to inline
// both bodies, so this stays a plain loop.
//
// The assertions do not measure time; they check that the output is still
// correct after the inlining change, and the shape is one that only becomes fast
// when the calls are gone.

fn main() {
    // The exact chain from the original reproducer: map over a range, collect
    // into a byte vector.
    let d: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    assert_eq!(d.len(), 1_000_000);
    assert_eq!(d[0], 0);
    assert_eq!(d[250], 250);
    assert_eq!(d[251], 0);
    println!("bytes {}", d.len());

    // The same shape with a wider element type and a non-trivial closure.
    let sq: Vec<u64> = (0..200_000u64).map(|i| i.wrapping_mul(i).wrapping_add(7)).collect();
    let mut acc = 0u64;
    for &x in &sq {
        acc = acc.wrapping_add(x);
    }
    println!("squares {}", acc);

    // A chained adapter: the fold body then calls through two closures.
    let evens: Vec<u64> = (0..300_000u64).filter(|i| i % 2 == 0).map(|i| i / 2).collect();
    assert_eq!(evens.len(), 150_000);
    assert_eq!(evens[0], 0);
    assert_eq!(evens[1], 1);
    println!("evens {} {}", evens.len(), evens[149_999]);

    // Iterator state that is not a simple range, so the loop cannot be turned
    // into an index walk: `step_by` keeps a counter in the iterator struct.
    let stepped: Vec<u64> = (0..100_000u64).step_by(7).collect();
    assert_eq!(stepped.len(), 14_286);
    println!("stepped {} {}", stepped.len(), stepped[14_285]);

    // Nested collect, where the inner chain is inlined into the outer closure.
    let nested: Vec<u64> = (0..2_000u64)
        .map(|i| (0..i.min(8)).map(|j| j + i).sum::<u64>())
        .collect();
    let total: u64 = nested.iter().copied().sum();
    println!("nested {} {}", nested.len(), total);
}
