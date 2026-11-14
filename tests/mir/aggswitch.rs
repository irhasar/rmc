#![no_std]
#![no_main]
include!("prelude.rs");

// A struct argument read into the return place, then matched on. rustc lowers
// the match to `switchInt(copy _0)`: the callee's `_0` is both the return value
// and the scrutinee. When the call is inlined, a discriminant mapped as a plain
// local would read the *caller's* `_0` (a stale slot) instead of the call
// destination, so every arm would take the `otherwise` path.
#[derive(Clone, Copy)]
struct P2 { x: i64, y: i64 }

#[inline(never)]
fn classify(p: P2) -> i64 {
    let mut s = p.y;
    match s {
        0 => s = s.wrapping_add(1),
        1 => s = s.wrapping_add(2),
        _ => s = 0,
    }
    s
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(1);
    // `y` selects the arm: 0 -> +1, 1 -> +2, anything else -> 0.
    out(classify(P2 { x: n, y: 0 }));
    out(classify(P2 { x: n, y: 1 }));
    out(classify(P2 { x: n, y: 7 }));
    // Also with the struct bound to a local first, so the argument is a copy.
    let p = P2 { x: n, y: 1 };
    out(classify(p));
    0
}
