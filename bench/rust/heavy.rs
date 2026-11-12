use std::env;
// Heavy load: a long-running (1e9 iteration) loop over two independent
// accumulators. The duration is in the hundreds of ms, so this is the
// steady-state regime where the throughput of the generated loop, not startup,
// decides the result. Two accumulators keep the loop from being closed into a
// closed form, and the data-dependent branch keeps it a scalar loop the
// vectoriser cannot rewrite; `alt` makes each iteration carry two live values
// so the loop cannot be reduced to a single running sum. As in `refs.rs`, there
// is deliberately no in-loop `black_box`: rmc lowers it to the identity while
// rustc makes it opaque, so an in-loop one would charge rustc a store/reload
// rmc never emits.
fn main() {
    let n: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000000000);
    let n = std::hint::black_box(n);
    let mut sum: i64 = 0;
    let mut alt: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        sum = sum + i;
        if sum > 1000000000 {
            sum -= 1000000000;
        }
        alt = alt + (i & 1);
        i += 1;
    }
    println!("{} {}", sum, alt);
}
