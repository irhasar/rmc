// A `Vec` binding's recorded stride is lexically scoped: a nested block that
// shadows `v` with a wider element type must not change the outer `v`'s stride
// once the block ends. Here the outer `v` is a byte vector, so `add(1)` steps a
// single byte even though an inner `v: Vec<i64>` came and went.
fn main() {
    let mut v: Vec<u8> = vec![0u8; 16];
    {
        let v: Vec<i64> = vec![0i64; 2];
        println!("{}", v.len());
    }
    let p = v.as_mut_ptr();
    let q = unsafe { p.add(1) };
    println!("{}", q as i64 - p as i64);

    // The reverse order: an inner byte vector under an outer word vector must
    // also restore the outer eight-byte stride.
    let mut w: Vec<i64> = vec![0i64; 4];
    {
        let mut w: Vec<u8> = vec![0u8; 4];
        let p = w.as_mut_ptr();
        let q = unsafe { p.add(3) };
        println!("{}", q as i64 - p as i64);
    }
    let p = w.as_mut_ptr();
    let q = unsafe { p.add(3) };
    println!("{}", q as i64 - p as i64);
}
