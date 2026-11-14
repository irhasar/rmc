#![no_std]
#![no_main]
include!("prelude.rs");

// Two inherent methods with the same name in different modules: each must get
// its own receiver-qualified symbol, or one body wins and both calls return it.
mod a {
    pub struct A { pub v: i64 }
    impl A { #[inline(never)] pub fn value(&self) -> i64 { 1 } }
}
mod b {
    pub struct B { pub v: i64 }
    impl B { #[inline(never)] pub fn value(&self) -> i64 { 2 } }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let x = a::A { v: 0 };
    let y = b::B { v: 0 };
    out(x.value());
    out(y.value());
    0
}
