#![no_std]
#![no_main]
include!("prelude.rs");

// A bounds check compares the index as *unsigned*: `-1 as usize` is
// `usize::MAX`, past the end of every array, and must panic (exit 101), not
// read the word before the array. Indexing happens in a loop whose body the
// trap leaves out of line, and both a frame array and one behind a reference
// are checked; the index reaches the code at run time through `arg`.
#[inline(never)]
fn through_ref(a: &[i64; 8], i: usize) -> i64 {
    a[i]
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let mut a: [i64; 8] = [0; 8];
    let mut j: usize = 0;
    while j < 8 {
        a[j] = k * j as i64;
        j += 1;
    }
    let mut s: i64 = 0;
    let mut n: i64 = 0;
    while n < 3 {
        let i = (k + n) as usize;
        s += a[i & 7];
        out(through_ref(&a, i));
        s += a[i];
        n += 1;
    }
    out(s);
    0
}
