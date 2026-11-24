// data-dependent single-diamond branch with a cheap arm (a `hash % 7 == 0`
// test): if-conversion should collapse the diamond into a select/cmove and
// remove the per-iteration mispredict.
fn main() {
    let mut acc: u64 = 1;
    let mut z: u64 = 0;
    for i in 0..30_000_000u64 {
        acc = acc.wrapping_mul(6364136223846793005).wrapping_add(i | 1);
        if acc % 7 == 0 {
            z ^= acc;
        }
    }
    println!("{}", z);
}
