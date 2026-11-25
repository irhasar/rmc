// An address smuggled through a bitcast must still make its slot escape: a
// non-inlined callee can reconstruct the address and read the slot. Provenance
// has to follow every format `pure_address_arith` accepts, including the
// conversions that are not `Binary` (`bitcast`, `uextend`, `ireduce`).
#[inline(never)]
fn read_through_bits(f: f64) -> u64 {
    let p = f.to_bits() as *const u64;
    let v = unsafe { *p };
    // Enough work that the callee is not worth inlining into `main`.
    v.wrapping_mul(3).wrapping_add(1)
}

fn main() {
    let mut x: u64 = 42;
    let addr = &mut x as *mut u64 as u64;
    // The caller only ever bitcasts the address to `f64`; the callee turns it
    // back into a pointer. Without provenance through the bitcast the slot looks
    // private and its initialization is dropped.
    let smuggled = f64::from_bits(addr);
    // 42*3 + 1 = 127.
    println!("{}", read_through_bits(smuggled));
}
