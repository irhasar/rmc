#![no_std]
#![no_main]
include!("prelude.rs");

// A generic struct in a *capitalised* module. A crate-root `G` shares the last
// path segment, so rustc's printer cannot shorten the module one and spells the
// literal `M::G::<i64> { .. }`. Stripping the generics leaves `M::G`, whose
// penultimate segment `M` is uppercase, so the capitalisation heuristic reads it
// as `Enum::Variant` and the back end writes a bogus tag before the fields. The
// struct's fields then land at offsets 8 and 16 instead of 0 and 8.
//
// `peek` reads through a reference so the aggregate is not constant folded, and
// the destination type is what must correct the parse-time guess.
struct G<T> {
    pub x: T,
}

mod M {
    pub struct G<T> {
        pub x: i64,
        pub y: T,
    }
}

#[inline(never)]
fn peek(g: &M::G<i64>) -> i64 {
    g.x + g.y
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(1);
    let g: M::G<i64> = M::G {
        x: a + 5,
        y: a + 100,
    };
    out(peek(&g));
    let h = G { x: a };
    out(h.x);
    0
}
