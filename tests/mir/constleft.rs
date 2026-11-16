#![no_std]
#![no_main]
include!("prelude.rs");

// A constant on the *left* of a non-commutative operator. The generator emits
// `mov $k, %rax; sub %rbx, %rax`, and a peephole once folded that into
// `lea -k(%rbx), %rax` - i.e. `x - k` instead of `k - x` - so `5 - x` printed
// `x - 5` and `0 - x` (`wrapping_neg`) printed `x`. `arg` keeps the operand
// opaque, or rustc folds the whole expression before the back end sees it.
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let x = arg(1);
    let y = arg(2);
    out(5 - x);
    out(0 - x);
    out(x.wrapping_neg());
    out(100 - y);
    out(-3 - x);
    out(7 + x);
    out(3 * x);
    out(1000 - x * y);
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < y {
        acc = 10 - acc;
        i += 1;
    }
    out(acc);
    out((1 << 4) - x);
    0
}
