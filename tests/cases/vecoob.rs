// A `Vec` index out of range must trap, exactly as rustc panics: stdout is
// empty and the exit status is 101. rmc used to lower `v[i]` to unchecked
// pointer arithmetic, so this read past the element store and printed garbage
// with exit 0 (CWE-125). The index comes from a parameter so it is not a
// constant the compiler could decide; the trap is a runtime branch.
fn read_at(i: i64) -> i64 {
    let v: Vec<i64> = vec![1, 2, 3];
    v[i as usize]
}

fn main() {
    println!("{}", read_at(5));
}
