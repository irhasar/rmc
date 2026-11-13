// A reference derived from a pointer must not be treated as frame storage.
//
// `r: &S` here is a null pointer. `let q = &r.inner` forms the address
// `*r + offset`; forming it does not read `r.inner`, so a function may do it
// even though `r` is null. The empty loop never runs, so the original program
// reads nothing. If LICM hoists `q.x` out of the loop because it mistakes
// `&r.inner` for a frame place, the hoisted load dereferences null and the
// program faults. rustc leaves the read in the loop and prints 0.
//
// The `unsafe` cast is deliberate: it is the shortest program both compilers
// accept (rmc's `0 as &S` extension is rejected by rustc as E0605).
struct I { x: i64 }
struct S { inner: I }

fn main() {
    let p: *const S = 0 as *const S;
    let r: &S = unsafe { &*p };
    let q: &I = &r.inner;
    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < 0 {
        acc += q.x;
        i += 1;
    }
    println!("{}", acc);
}
