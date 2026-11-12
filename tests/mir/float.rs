#![no_std]
#![no_main]
include!("prelude.rs");

// `f64`/`f32` arithmetic, comparison and a float-accumulating loop. `out`
// prints an i64, so each result is cast; that is enough to catch a wrong
// rounding mode, a swapped operand or a mis-lowered comparison, which are the
// ways a hand-written float path fails. This case is why the back end must not
// claim "floating-point types are not implemented" (see `classify_unsupported`).
#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = 1.5f64;
    let b = 2.25f64;
    out((a + b) as i64);       // 3
    out((b - a) as i64);       // 0
    out((a * b) as i64);       // 3
    out((b / a) as i64);       // 1
    out((a * 4.0) as i64);     // 6
    out((0.0 - a) as i64);     // -1 (truncates toward zero)

    // Comparison must yield a real bool, and it must branch.
    outb(a < b);
    outb(a > b);
    outb(a == 1.5);
    outb(a != b);

    // A float loop with a data-dependent step, so the value cannot be folded.
    let n = arg(1);
    let mut s = 0.0f64;
    let mut i = 0i64;
    while i < n {
        s += (i as f64) * 0.5;
        i += 1;
    }
    out(s as i64);

    // `f32` is a separate width; a load/store that used the wrong size shows
    // up here even when the f64 path is correct.
    let c = 3.5f32;
    let d = 2.0f32;
    out((c * d) as i64);       // 7
    out((c - d) as i64);       // 1

    // A float field in a struct, read back through a value and a reference.
    let p = Pair { x: 1.25f64, y: 8.0f64 };
    out((p.x * p.y) as i64);   // 10
    let r = &p;
    out((r.y / r.x) as i64);   // 6
    0
}

struct Pair {
    x: f64,
    y: f64,
}
