// Regression guard for u64/usize -> float conversion in a loop.
//
// `cvtsi2s{s,d}` is a two-operand SSE instruction that merges the upper bits
// of its destination. Without a preceding `xorps` the conversion carries a
// false dependency on whatever last wrote the destination register, which in
// a loop serializes iterations behind the previous call's result.
//
// This kernel isolates the unsigned path: the call result is what the
// destination register holds when the next conversion executes, so a missing
// dependency break shows up as a large slowdown rather than a rounding error.
fn main() {
    let n = 4_000_000usize;
    let mut a = vec![0f32; n];
    for i in 0..n {
        a[i] = (i as f32).sin();
    }
    println!("{}", a[12345]);
}
