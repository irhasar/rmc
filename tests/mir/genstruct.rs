#![no_std]
#![no_main]
include!("prelude.rs");

// A generic struct literal with a turbofish (`G::<T>`) must stay a plain
// aggregate. rustc prints an enum variant as `<enum>::<Variant>` and a generic
// struct as `<struct>::<args>`, so the two share the shape `Name::suffix`. The
// reclassifier has to strip the generics *before* looking for the variant
// segment: `G::<T>` reduces to `G` (a struct), while `Option::<i64>::Some`
// reduces to `Option::Some` (a variant). Splitting the raw name would take the
// turbofish as the variant and write a phantom tag word, shifting every field
// by one and corrupting the value.
//
// `peek` reads the value through a reference so the aggregate is not constant
// folded away, and the module-qualified type argument keeps the printed path
// from collapsing to a bare name.

mod m { pub struct T { pub v: i64 } }
struct G<T> { x: i64, p: T }

#[inline(never)]
fn peek(g: &G<m::T>) -> i64 { g.x + g.p.v }

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(1);
    let g: G<m::T> = G { x: a + 5, p: m::T { v: a + 100 } };
    out(peek(&g));
    0
}
