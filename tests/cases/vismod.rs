// `pub` may prefix an item (`fn`, `struct`, `enum`, `const`, `type`), a struct
// field, an `impl` method and a nested `fn`. rmc resolves every name in one flat
// namespace, so the visibility is dropped; accepting it lets an ordinary Rust
// source file parse unchanged. An inline `mod NAME { .. }` is skipped the same
// way. The twin prints the values so both compilers write the same bytes.
pub const N: i64 = 10;
pub type T = i64;

pub struct Point {
    pub x: i64,
    pub y: i64,
}

pub enum Color {
    Red,
    Green,
}

impl Point {
    pub fn sum(&self) -> i64 {
        self.x + self.y
    }
}

pub mod outer {
    pub fn hidden() -> i64 {
        99
    }
}

pub fn scale(v: T) -> i64 {
    v * 2
}

fn main() {
    let p = Point { x: 3, y: 4 };
    println!("{}", p.sum());
    println!("{}", scale(N));
    let c = Color::Green;
    println!("{}", c as i64);
    pub fn nested() -> i64 {
        7
    }
    println!("{}", nested());
}
