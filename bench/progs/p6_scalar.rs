// Plain scalar while-loop, no iterator struct, no function calls.
// Isolates whether Rakit promotes MIR locals to registers.
fn main() {
    let mut acc: u64 = 0;
    let mut i: u64 = 0;
    while i < 300_000_000u64 {
        acc = acc.wrapping_add(i).wrapping_mul(3);
        i += 1;
    }
    println!("{}", acc);
}
