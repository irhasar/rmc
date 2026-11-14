#![no_std]
#![no_main]
include!("prelude.rs");

// Two distinct types that share a last path segment (`a::A` and
// `outer::a::A`), each with an inherent method of the same name. The printer
// spells the first bare in its own module and the second fully qualified, so
// the back end must neither merge them nor give the module-qualified struct a
// phantom enum tag.

mod a {
    pub struct A {
        pub x: i64,
    }
    impl A {
        #[inline(never)]
        pub fn value(&self) -> i64 {
            self.x
        }
    }
}

mod outer {
    pub mod a {
        pub struct A {
            pub x: i64,
            pub y: i64,
        }
        impl A {
            #[inline(never)]
            pub fn value(&self) -> i64 {
                self.x + self.y
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let p = a::A { x: arg(1) };
    let q = outer::a::A { x: arg(2), y: arg(3) };
    out(p.value());
    out(q.value());
    0
}
