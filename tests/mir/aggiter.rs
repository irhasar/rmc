#![no_std]
#![no_main]
include!("prelude.rs");

// A generic struct literal prints with its generics (`Iter::<'_, i64>`), not as
// a variant path. Reading it as an enum variant added a phantom tag word and
// shifted both fields, so a two-field struct clobbered its neighbour's frame.
struct Pair<T> {
    a: T,
    b: T,
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let p = Pair::<i64> { a: 11, b: 22 };
    out(p.a);
    out(p.b);
    // A second local right after the two-field struct, so a one-word region
    // would overlap it.
    let mut s = 0i64;
    s += p.b;
    out(s);
    0
}
