#![no_std]
#![no_main]
include!("prelude.rs");

struct G { v: i64 }
impl Drop for G {
    fn drop(&mut self) { out(self.v); }
}

// Eight scalars live across the drop of `x`. A non-allocating function promotes
// them into the five callee-saved registers first, then into the caller-saved
// `%rsi/%rdi/%r8/%r9`, so the later ones land in caller-saved registers. The
// destructor call clobbers those: the spill must precede the `mov %rax, %rdi`
// that passes the address, or a promoted `%rdi` is overwritten before it is
// saved and the value is lost on reload.
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
        let x = G { v: a + b + c + d + e + f + g + h };
        out(x.v);
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
