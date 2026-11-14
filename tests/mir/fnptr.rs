#![no_std]
#![no_main]
include!("prelude.rs");

// A function pointer is now a *supported* case: taking a function as a value
// reifies its address (`ReifyFnPointer`), and a call through a `fn`-typed local
// is an indirect `call *%reg`. The reference is rustc; the two programs must
// print identical output.

fn add(a: i64, b: i64) -> i64 {
    a + b
}
fn mul(a: i64, b: i64) -> i64 {
    a * b
}
fn apply(f: fn(i64, i64) -> i64, a: i64, b: i64) -> i64 {
    f(a, b)
}
fn pick(which: i64) -> fn(i64, i64) -> i64 {
    if which == 0 {
        add
    } else {
        mul
    }
}

struct Ops {
    op: fn(i64, i64) -> i64,
    bias: i64,
}

fn run_op(o: &Ops, a: i64, b: i64) -> i64 {
    (o.op)(a, b) + o.bias
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    // A `fn` pointer passed as an argument and called indirectly.
    let f: fn(i64, i64) -> i64 = add;
    out(apply(f, 10, 3));
    // Two distinct pointers to the same signature, called in turn.
    let g: fn(i64, i64) -> i64 = mul;
    out(apply(g, 10, 3));
    // A pointer *returned* from a function, selected at runtime.
    out(apply(pick(0), 6, 7));
    out(apply(pick(1), 6, 7));
    // A pointer stored in a struct field and called through it.
    let o = Ops { op: add, bias: 100 };
    out(run_op(&o, 1, 2));
    // An array of pointers, indexed at compile time (a runtime index would
    // need `panic_bounds_check`, which the freestanding reference does not
    // link; the array-index path itself is covered by `arrays`).
    let tbl: [fn(i64, i64) -> i64; 2] = [add, mul];
    out(apply(tbl[0], 5, 5));
    out(apply(tbl[1], 5, 5));
    0
}
