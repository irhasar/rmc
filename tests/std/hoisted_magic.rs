// `x % 1000003` in a loop hoists the division's magic multiplier into a
// callee-saved register, loaded in the prologue, and copies it into `%rcx` for
// the one-operand `imul`. The `coalesce_scratch` peephole deleted that copy and
// renamed the source register to `%rcx` instead, so `imul %rcx` multiplied by
// whatever `%rcx` held and the sum came out as garbage.
use std::hint::black_box;

#[inline(never)]
fn h(n: i64, k: i64) -> i64 {
    let mut s: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        s += (i * k) % 1000003;
        i += 1;
    }
    s
}

// One call, so rmc's inliner folds `h` into `main`, where the constant takes
// a callee-saved register of its own.
fn main() {
    println!("{}", h(black_box(1000), black_box(2654435761)));
}
