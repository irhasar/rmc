// A helper that accumulates into a local which rustc promotes to the return
// place (`_0 = Add(copy _0, move _5)`), then is inlined by rmc. The inliner
// must redirect the callee's `_0` to the caller's destination in *every*
// operand, not only the final `Use`: reading it as an ordinary local aliases
// the caller's own `_0` and the sum collapses to its last term. The arguments
// are `&[i64]`, so this also exercises array-to-slice unsizing at the call.
fn sum(xs: &[i64]) -> i64 {
    let mut s = 0;
    let mut i = 0;
    while i < xs.len() {
        s += xs[i];
        i += 1;
    }
    s
}

fn main() {
    let a = [2i64, 3, 5, 7, 11, 13];
    println!("{}", sum(&a));
    let b = [10i64, 20, 30];
    println!("{}", sum(&b));
}
