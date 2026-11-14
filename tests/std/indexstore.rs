// `arr[i] = v` with a computed index and a computed value. `copy_propagate`
// rewrote operand reads through its alias map but not a place's dynamic index,
// and it folded away the index local's single definition: the index slot was
// left with no writer, so the store addressed element 0. `dst[i] = src[i]`
// wrote every element to `dst[0]`.
fn main() {
    let src = [10i64, 20, 30, 40];
    let mut dst = [0i64; 4];
    let mut i = 0usize;
    while i < 4 {
        dst[i] = src[i];
        i += 1;
    }
    println!("{} {} {} {}", dst[0], dst[1], dst[2], dst[3]);

    // The value may come from a place read (`*p`) rather than another element.
    let p = &42i64;
    let mut arr = [0i64; 3];
    let mut j = 0usize;
    while j < 3 {
        arr[j] = *p + j as i64;
        j += 1;
    }
    println!("{} {} {}", arr[0], arr[1], arr[2]);

    // A constant index still works, and a store into a slice-like view.
    let mut k = [7i64, 8, 9];
    k[1] = -1;
    println!("{} {} {}", k[0], k[1], k[2]);

    // An index expression (`i + 1`) is its own temporary; it must survive too.
    let mut m = [0i64; 5];
    let mut n = 0usize;
    while n < 4 {
        m[n + 1] = (n as i64) * 10;
        n += 1;
    }
    println!("{} {} {} {} {}", m[0], m[1], m[2], m[3], m[4]);
}
