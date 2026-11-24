// Regression test for the atom-aware affine reassociation pass (REPORT §19).
// Each loop is an LCG whose forcing term is a bitwise op (`i|1`, `i&k`, `i^k`)
// combined with a reduction (`min`, xor, add). The pass must reassociate the
// carried multiply chain while keeping every value exact, so the output must
// match LLVM byte-for-byte.
//
// The unroller only fires on a compile-time-constant trip count, so the bounds
// are constants. A reduction state (min/xor/add) stops the loop from folding to
// a closed form, so these actually run and exercise the atom path.
fn main() {
    const N: u64 = 2_000_000;
    let c: u64 = 6364136223846793005;

    // min-reduction over `acc = acc*C + (i|1)` (p8 shape)
    let mut a = 1u64;
    let mut mn = u64::MAX;
    for i in 0..N {
        a = a.wrapping_mul(c).wrapping_add(i | 1);
        if a < mn {
            mn = a;
        }
    }

    // xor-reduction over `acc = acc*C + (i & 0xff)`
    let mut b = 1u64;
    let mut z = 0u64;
    for i in 0..N {
        b = b.wrapping_mul(c).wrapping_add(i & 0xff);
        z ^= b;
    }

    // add-reduction over `acc = acc*C + (i ^ 0x5a5a)`
    let mut d = 1u64;
    let mut s = 0u64;
    for i in 0..N {
        d = d.wrapping_mul(c).wrapping_add(i ^ 0x5a5a);
        s = s.wrapping_add(d);
    }

    // pure affine reduction (no atom) as a control
    let mut e = 0u64;
    let mut t = 0u64;
    for i in 0..N {
        e = e.wrapping_mul(c).wrapping_add(i);
        t = t.wrapping_add(e);
    }

    println!("{} {} {} {} {}", mn, z, s, t, a ^ b ^ d ^ e);
}
