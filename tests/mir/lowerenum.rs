#![no_std]
#![no_main]
include!("prelude.rs");

// A lowercase enum name (legal, if unidiomatic). The back end cannot decide a
// brace aggregate is a variant from a capitalised path segment, so it must use
// the destination's declared type instead. `A` takes tag 0 and `Some` tag 1, so
// a missing discriminant on `e::Some { .. }` reads `x` where the tag should be
// and, for `x = 0`, reports `A`.
#[allow(non_camel_case_types)]
enum e {
    A,
    Some { x: i64 },
}

#[inline(never)]
fn consume(v: e) -> i64 {
    match v {
        e::A => 777,
        e::Some { x } => x,
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let sel = arg(1);
    let v = if sel > 0 { e::Some { x: arg(2) } } else { e::A };
    out(consume(v));
    0
}
