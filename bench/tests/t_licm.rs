// Regression coverage for the LICM pass.
//
// Each function targets one shape that broke an earlier version of the pass:
// loop-carried block parameters (phi nodes), a conditional whose both arms
// re-enter the header, and stores that must block hoisting of a load. The
// expected values are computed independently, so a wrong hoist changes the
// output rather than merely slowing it down.

fn phi_carried(n: u64) -> u64 {
    // `acc` and `i` are header block parameters: treating either as invariant
    // at the preheader yields the entry value forever.
    let mut acc = 7u64;
    let mut i = 0u64;
    while i < n {
        acc = acc.wrapping_mul(3).wrapping_add(i);
        i += 1;
    }
    acc
}

fn both_arms_reenter(n: u64) -> u64 {
    // A conditional whose two arms both continue the loop. If only one edge is
    // retargeted at the preheader, the other bypasses the hoisted code.
    let mut acc = 0u64;
    let mut i = 0u64;
    while i < n {
        if i % 2 == 0 {
            acc = acc.wrapping_add(i);
            i += 1;
        } else {
            acc = acc.wrapping_sub(i);
            i += 1;
        }
    }
    acc
}

fn store_blocks_hoist(n: usize) -> u64 {
    // The load of `v[0]` is not invariant: the loop stores to `v`.
    let mut v = vec![1u64, 2, 3, 4];
    let mut acc = 0u64;
    let mut i = 0usize;
    while i < n {
        acc = acc.wrapping_add(v[0]);
        v[0] = v[0].wrapping_add(1);
        i += 1;
    }
    acc.wrapping_add(v[0])
}

fn invariant_load(n: usize) -> u64 {
    // `scale`/`bias` live outside the loop and the loop only writes `out`, so
    // these loads are invariant and safe to hoist.
    const SCALE: u64 = 2654435761;
    const BIAS: u64 = 12345;
    let mut out = vec![0u64; n];
    let mut i = 0usize;
    while i < n {
        out[i] = (i as u64).wrapping_mul(SCALE).wrapping_add(BIAS);
        i += 1;
    }
    out.iter().copied().fold(0u64, u64::wrapping_add)
}

fn trapping_not_hoisted(n: u64) -> u64 {
    // Division by a loop-varying denominator may trap. Hoisting it out of the
    // loop would execute it when the loop body never runs.
    let mut acc = 0u64;
    let mut i = 1u64;
    while i < n {
        acc = acc.wrapping_add(1000 / i);
        i += 1;
    }
    acc
}

fn main() {
    let n = 1000u64;
    println!("{}", phi_carried(n));
    println!("{}", both_arms_reenter(n));
    println!("{}", store_blocks_hoist(n as usize));
    println!("{}", invariant_load(n as usize));
    println!("{}", trapping_not_hoisted(n));
    println!("{}", trapping_not_hoisted(1));
}
