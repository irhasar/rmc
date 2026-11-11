// `add(off)` on a raw pointer steps whole elements, so a `Vec<i64>` pointer
// advances eight bytes per element. A wrong (byte) stride makes element k alias
// element 8k, so the writes overwrite each other.
fn main() {
    let mut v: Vec<i64> = vec![0i64; 8];
    let base = v.as_mut_ptr();
    let mut i: i64 = 0;
    while i < 8 {
        let p = unsafe { base.add(i as usize) } as *mut i64;
        unsafe { *p = i * i; }
        i += 1;
    }
    let mut sum: i64 = 0;
    let mut i: i64 = 0;
    while i < 8 {
        let p = unsafe { base.add(i as usize) } as *mut i64;
        sum += unsafe { *p };
        i += 1;
    }
    println!("{}", sum);
}
