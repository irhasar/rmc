#![no_std]
#![no_main]
include!("prelude.rs");

// A `Result` whose payload the compiler cannot const-fold: `f` picks the
// variant from a runtime value, so the MIR really does build the aggregate and
// read the payload back out through `(x as Ok).0`.
//
// The two front ends spell the enum differently. `--emit=mir` prints a
// construction as the bare `Result::<i64, i64>::Ok`, but the local's declared
// type as the qualified `core::result::Result<i64, i64>`. When the two
// spellings were not reconciled the enum layout was learned under the bare
// `Result` while the payload projection looked it up under the qualified name,
// found nothing, and used offset 0 - the tag - so an `Ok` read back its own
// discriminant (0 here) instead of the payload. This pins that down: both
// payload widths (a scalar and a three-word aggregate) must read back.
struct Big {
    a: i64,
    b: i64,
    c: i64,
}

fn small(x: i64) -> Result<i64, i64> {
    if x > 0 {
        Ok(x)
    } else {
        Err(x)
    }
}

fn big(x: i64) -> Result<i64, Big> {
    if x > 0 {
        Ok(x)
    } else {
        Err(Big { a: x, b: x, c: x })
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(1);
    if let Ok(x) = small(n) {
        out(x);
    } else {
        out(99);
    }
    if let Ok(x) = big(n) {
        out(x);
    } else {
        out(98);
    }
    0
}
