#[inline(never)]
fn record(value: u64) {
    println!("{value}");
}

fn main() {
    let mut acc: u64 = 1;
    for i in 0..8u64 {
        record(acc);
        acc = acc.wrapping_mul(3).wrapping_add(i);
    }
    println!("acc={acc}");
}
