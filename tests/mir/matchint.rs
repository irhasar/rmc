#![no_std]
#![no_main]
include!("prelude.rs");

fn classify(n: i64) -> i64 {
    match n {
        0 => 100,
        1 | 2 => 200,
        3..=5 => 300,
        _ => 400,
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(classify(0));
    out(classify(2));
    out(classify(4));
    out(classify(9));
    out(classify(arg(1)));
    0
}
