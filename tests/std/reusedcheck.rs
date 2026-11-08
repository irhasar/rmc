// rustc reuses one `bool` local for the bounds check of a loop that indexes the
// same array twice per iteration. The first `assert` folds the comparison into
// a `cmp`/`jcc`; if the backend also drops the `bool` store, the second `assert`
// reads a stale slot and traps. This is the shape that caught it.
fn main() {
    let v = [1, 3, 5, 7, 9];
    let t = 7;
    let mut lo = 0usize;
    let mut hi = v.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        println!("{} {}", mid, v[mid]);
        if v[mid] < t {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    println!("end");
}
