use std::env;
// Light load: a 2 KiB table that stays in L1, scanned with a data-dependent
// branch. The table is small enough to never leave cache and the run is short
// (a few ms), so per-iteration codegen and process startup dominate. The branch
// on `acc` keeps the loop a real per-iteration loop: it defeats LLVM's
// loop-closing (`n * K`) and its vectoriser, so both compilers run a scalar
// loop of the same shape and the ratio reflects the generated code rather than
// a barrier one side pays for and the other does not. There is deliberately no
// in-loop `black_box`: rmc lowers it to the identity while rustc makes it
// opaque, so an in-loop one would charge rustc a store/reload rmc never emits.
fn main() {
    let n: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000000);
    let n = std::hint::black_box(n);
    let mut table = [0i64; 256];
    let mut i: i64 = 0;
    while i < 256 {
        table[i as usize] = i.wrapping_mul(2654435761) % 1000003;
        i += 1;
    }
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let v = table[(i & 255) as usize];
        acc = acc + v;
        if acc > 1000000 {
            acc -= 1000000;
        }
        i += 1;
    }
    println!("{}", acc);
}
