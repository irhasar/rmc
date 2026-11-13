// A tuple struct field may be a multi-token type (`&T`, `*const T`, a nested
// tuple or an array). The parser's field-count prescan must step over the whole
// type, not one token, or `struct P(&i64, i64)` is mis-counted and rejected.
struct R(&'static i64, i64);
struct Raw(*const i64, i64);
struct Nested((i64, i64), [i64; 2]);
struct Mixed(*const i64, &'static i64);

fn main() {
    // A reference field holds a promoted constant.
    let r = R(&42, 3);
    println!("{} {}", *r.0, r.1);

    let x: i64 = 5;
    let raw = Raw(&x as *const i64, 4);
    println!("{} {}", raw.1, unsafe { *raw.0 });

    let n = Nested((1, 2), [7, 8]);
    println!("{} {} {} {}", n.0 .0, n.0 .1, n.1[0], n.1[1]);

    let m = Mixed(&x as *const i64, &42);
    println!("{} {}", unsafe { *m.0 }, *m.1);
}
