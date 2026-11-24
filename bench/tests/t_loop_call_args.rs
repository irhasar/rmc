#[inline(never)]
fn record(value: u64, seen: &mut Vec<u64>) {
    seen.push(value);
    println!("{value}");
}

fn main() {
    let mut acc: u64 = 1;
    let mut seen = Vec::new();
    for i in 0..8u64 {
        record(acc, &mut seen);
        acc = acc.wrapping_mul(3).wrapping_add(i);
    }
    assert_eq!(seen, [1, 3, 10, 32, 99, 301, 908, 2730]);
    assert_eq!(acc, 8197);
    println!("acc={acc}");
}
