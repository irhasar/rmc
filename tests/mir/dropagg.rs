#![no_std]
#![no_main]
include!("prelude.rs");

// Dropping an aggregate runs its destructors through `%rdi`, the first argument
// register. A promoted caller-saved local can itself live in `%rdi`, so the
// spill that preserves it must happen before the glue materialises any address
// into `%rdi`; otherwise the local is saved as an address and reloaded as one.
// The array, the nested struct and the payload enum each exercise a different
// glue entry path (field loops, nested field offsets, tag dispatch).

struct G { v: i64 }
impl Drop for G {
    fn drop(&mut self) { out(self.v); }
}

struct Wrap { a: G, b: i64, c: G }

enum E { A(G), B(i64, G) }

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let a = arg(1);
    let b = arg(2);
    let c = arg(3);
    let d = arg(4);
    let e = arg(5);
    let f = arg(6);
    let g = arg(7);
    let h = arg(8);
    {
        let arr = [G { v: 100 }, G { v: 200 }, G { v: 300 }];
        out(arr[0].v + arr[1].v + arr[2].v);
    }
    {
        let w = Wrap { a: G { v: a + 400 }, b: 0, c: G { v: a + 500 } };
        out(w.a.v + w.c.v);
    }
    {
        let x = E::B(a + 600, G { v: a + 700 });
        match x {
            E::A(_) => out(0),
            E::B(n, gg) => out(n + gg.v),
        }
    }
    out(a);
    out(b);
    out(c);
    out(d);
    out(e);
    out(f);
    out(g);
    out(h);
    0
}
