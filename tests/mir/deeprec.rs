#![no_std]
#![no_main]
include!("prelude.rs");

fn sumto(n: i64) -> i64 {
    if n == 0 {
        0
    } else {
        n + sumto(n - 1)
    }
}

fn ack(m: i64, n: i64) -> i64 {
    if m == 0 {
        n + 1
    } else if n == 0 {
        ack(m - 1, 1)
    } else {
        ack(m - 1, ack(m, n - 1))
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(sumto(100));
    out(ack(2, 3));
    0
}
