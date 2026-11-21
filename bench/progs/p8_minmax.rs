// min-reduction: classic select/cmove win; also exercises the cost model,
// since the condition is data-dependent and the arm is one cheap `cmov`.
fn main() {
    let mut a: u64 = 1;
    let mut mn: u64 = u64::MAX;
    for i in 0..30_000_000u64 {
        a = a.wrapping_mul(6364136223846793005).wrapping_add(i | 1);
        if a < mn {
            mn = a;
        }
    }
    println!("{}", mn);
}
