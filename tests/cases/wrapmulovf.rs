// `wrapping_mul` is `*` without the overflow trap, so under `--overflow-checks`
// it must still wrap rather than trap. The operands are runtime values (from
// argv) so the fold cannot fold the product away, and the overflowing pairs
// would trap a plain `*` under `-C overflow-checks=on`; `wrapping_mul` must
// instead agree with the reference on the wrapped product.
fn main() {
    let a: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let b: i64 = std::env::args().nth(2).unwrap().parse().unwrap();
    println!("{}", a.wrapping_mul(b));
}
