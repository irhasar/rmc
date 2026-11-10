#![no_std]
#![no_main]
include!("prelude.rs");

struct G { v: i64 }
impl Drop for G {
    fn drop(&mut self) { out(self.v); }
}

// No `Drop` impl of its own: dropping it runs the glue for its fields, calling
// the nested `G` impl and then each droppable array element.
struct Outer { g: G, k: i64 }
struct Arr { gs: [G; 2] }

// An impl of its own *and* a droppable field: the impl runs first, then the
// field's destructor (rustc's `drop_in_place` order).
struct Wrap { g: G }
impl Drop for Wrap {
    fn drop(&mut self) { out(100); }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let g = G { v: 7 };
    let o = Outer { g, k: 1 };
    out(o.k);
    let a0 = G { v: 8 };
    let a1 = G { v: 9 };
    let arr = Arr { gs: [a0, a1] };
    out(arr.gs[0].v);
    let w = Wrap { g: G { v: 5 } };
    out(w.g.v);
    0
}
