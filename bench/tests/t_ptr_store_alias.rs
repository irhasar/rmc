// A store through a pointer that is *also* an operand of further address
// arithmetic is untracked by store forwarding, so it must not leave the slot
// private: otherwise a later direct read of the overwritten element is
// forwarded the stale initialization.
fn main() {
    let mut arr = [0u64; 4];
    // 0 at run time, unknown to the compiler.
    let off: usize = std::env::args().count() - 1;
    unsafe {
        let sa = arr.as_mut_ptr();
        *sa = 1;
        let p = sa.add(off);
        *p = 7; // overwrites arr[0] through a derived pointer
        let q = p.add(1); // `p` also feeds address arithmetic
        // Both reads happen here: passing the places straight to `println!`
        // would hand the address to `core::fmt` and make the slot escape.
        let a = *sa;
        let b = *q;
        println!("{} {}", a, b);
    }
}
