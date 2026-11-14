#![no_std]
#![no_main]
include!("prelude.rs");

// A recursive, non-inlinable aggregate-returning function. rustc lowers the
// `P` return as a hidden pointer in `%rdi`; the caller must pass the address of
// its destination and every real argument shifts one register along (`%rsi`
// for argument 0). `aggret.rs` only covers a function rustc inlines, so this is
// the case that exercises the sret ABI.
struct P {
    x: i64,
    y: i64,
}

#[inline(never)]
fn build(n: i64) -> P {
    if n == 0 {
        P { x: 1, y: 1 }
    } else {
        let p = build(n - 1);
        P { x: p.x + 1, y: p.y * 2 }
    }
}

#[inline(never)]
fn pair(a: i64, b: i64, c: i64, d: i64) -> P {
    P { x: a + b, y: c + d }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(4);
    let p = build(n);
    out(p.x);
    out(p.y);
    let q = pair(n, n + 1, n + 2, n + 3);
    out(q.x);
    out(q.y);
    0
}
