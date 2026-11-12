// A `vec![a, b, c]` list allocates one slot per element and stores each, so
// reading them back through a pointer yields the written values. A `u8`
// element occupies one byte and a wider element one word, and `.len()` /
// `.capacity()` fold to the element count.
fn main() {
    let v: Vec<i64> = vec![11, 22, 33];
    let p = v.as_ptr();
    let b0 = p as *const i64;
    let b1 = unsafe { p.add(1) } as *const i64;
    let b2 = unsafe { p.add(2) } as *const i64;
    println!("{}", v.len());
    println!("{}", v.capacity());
    println!("{} {} {}", unsafe { *b0 }, unsafe { *b1 }, unsafe { *b2 });

    let u: Vec<u8> = vec![7u8, 8u8, 200u8];
    let q = u.as_ptr();
    let c0 = q as *const u8;
    let c1 = unsafe { q.add(1) } as *const u8;
    let c2 = unsafe { q.add(2) } as *const u8;
    println!("{}", u.len());
    println!("{} {} {}", unsafe { *c0 }, unsafe { *c1 }, unsafe { *c2 });

    let single: Vec<i64> = vec![42];
    let sp = single.as_ptr();
    let s = sp as *const i64;
    println!("{} {}", single.len(), unsafe { *s });

    let empty: Vec<i64> = vec![0; 0];
    println!("{}", empty.len());
}
