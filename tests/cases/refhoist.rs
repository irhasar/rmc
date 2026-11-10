// The reference field is null and the loop
// never runs, so the read is never performed and the program prints 7.
struct S { n: i64 }
struct Holder { p: *const S }

fn main() {
    let h = Holder { p: std::ptr::null() };
    let mut out: i64 = 7;
    while false {
        out = unsafe { (*h.p).n };
    }
    println!("{}", out);
}
