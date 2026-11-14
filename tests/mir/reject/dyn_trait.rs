#![no_std]
#![no_main]
include!("../prelude.rs");

// A trait object is a *rejected* case: the vtable and the virtual call are not
// implemented, and the rejection must be a clean error naming `dyn Trait`, not
// a crash or a stray `undefined symbol` from the linker. rustc compiles this
// (the reference build succeeds), so the suite can assert both sides: a working
// program on the reference, a named error from rmc.
trait Shape {
    fn area(&self) -> i64;
}

struct Sq {
    s: i64,
}

impl Shape for Sq {
    fn area(&self) -> i64 {
        self.s * self.s
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let q = Sq { s: arg(1) };
    let sh: &dyn Shape = &q;
    out(sh.area());
    0
}
