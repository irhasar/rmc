#![no_std]
#![no_main]
include!("prelude.rs");

// rustc spells a type *relative to the item that mentions it*. Inside `outer`,
// the nested `outer::a::A` prints as `a::A`, while at the crate root the same
// type prints as `outer::a::A`. A qualified spelling like `a::A` looks like a
// complete path, so the bare-name reconciliation skips it and the callee keys a
// *different* type than the caller built. Its layout is then unknown, field
// reads fall back to `k*8`, and `v.i.q` / `v.x` (real offsets 8 and 16, past a
// two-word `Inner`) read the wrong words.
//
// The sibling `outer::A` is what stops the printer from shortening the nested
// type: without it `outer::a::A` would print bare as `A`, which the bare-name
// pass already handles. The callee's match is large enough that the inliner
// leaves it in place, so its own relatively-spelled body is the code that runs.
mod outer {
    pub struct A {
        pub x: i64,
    }
    pub mod a {
        pub struct Inner {
            pub p: i64,
            pub q: i64,
        }
        pub struct A {
            pub i: Inner,
            pub x: i64,
        }
    }

    #[inline(never)]
    pub fn get(v: a::A) -> i64 {
        let mut s = v.i.q;
        match v.i.p {
            0 => s = s.wrapping_add(v.x).wrapping_add(0),
            1 => s = s.wrapping_add(v.x).wrapping_add(1),
            2 => s = s.wrapping_add(v.x).wrapping_add(2),
            3 => s = s.wrapping_add(v.x).wrapping_add(3),
            4 => s = s.wrapping_add(v.x).wrapping_add(4),
            5 => s = s.wrapping_add(v.x).wrapping_add(5),
            6 => s = s.wrapping_add(v.x).wrapping_add(6),
            7 => s = s.wrapping_add(v.x).wrapping_add(7),
            8 => s = s.wrapping_add(v.x).wrapping_add(8),
            9 => s = s.wrapping_add(v.x).wrapping_add(9),
            10 => s = s.wrapping_add(v.x).wrapping_add(10),
            11 => s = s.wrapping_add(v.x).wrapping_add(11),
            12 => s = s.wrapping_add(v.x).wrapping_add(12),
            13 => s = s.wrapping_add(v.x).wrapping_add(13),
            14 => s = s.wrapping_add(v.x).wrapping_add(14),
            15 => s = s.wrapping_add(v.x).wrapping_add(15),
            16 => s = s.wrapping_add(v.x).wrapping_add(16),
            17 => s = s.wrapping_add(v.x).wrapping_add(17),
            18 => s = s.wrapping_add(v.x).wrapping_add(18),
            19 => s = s.wrapping_add(v.x).wrapping_add(19),
            20 => s = s.wrapping_add(v.x).wrapping_add(20),
            21 => s = s.wrapping_add(v.x).wrapping_add(21),
            22 => s = s.wrapping_add(v.x).wrapping_add(22),
            23 => s = s.wrapping_add(v.x).wrapping_add(23),
            24 => s = s.wrapping_add(v.x).wrapping_add(24),
            25 => s = s.wrapping_add(v.x).wrapping_add(25),
            26 => s = s.wrapping_add(v.x).wrapping_add(26),
            27 => s = s.wrapping_add(v.x).wrapping_add(27),
            28 => s = s.wrapping_add(v.x).wrapping_add(28),
            29 => s = s.wrapping_add(v.x).wrapping_add(29),
            30 => s = s.wrapping_add(v.x).wrapping_add(30),
            31 => s = s.wrapping_add(v.x).wrapping_add(31),
            _ => s = 0,
        }
        s.wrapping_add(v.x)
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let t = outer::A { x: arg(1) };
    let v = outer::a::A {
        i: outer::a::Inner { p: arg(2), q: arg(3) },
        x: arg(4),
    };
    out(t.x);
    out(outer::get(v));
    0
}
