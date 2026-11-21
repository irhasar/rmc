use std::cell::RefCell;
#[inline(never)]
fn observe(p: &u64) -> u64 { *p }
#[inline(never)]
fn clobber(p: &mut u64, v: u64) { *p = v; }

struct Pair { a: u64, b: u64 }
impl Pair {
    fn swap(&mut self) { let t = self.a; self.a = self.b; self.b = t; }
    fn sum(&self) -> u64 { self.a.wrapping_add(self.b) }
}

fn tuple_roundtrip(x: u64, y: u64) -> u64 {
    let t = (x.wrapping_mul(3), y.wrapping_add(7));
    let (a, b) = t;
    a.wrapping_mul(b)
}

fn opt_roundtrip(v: Option<u64>) -> u64 {
    let mut o = v;
    for i in 0..100u64 { o = o.map(|x| x.wrapping_add(i)); }
    o.unwrap_or(0)
}

fn main() {
    let mut acc = 0u64;
    // store then read through an escaping pointer
    let mut x = 5u64;
    clobber(&mut x, 42);
    acc = acc.wrapping_add(observe(&x));
    // struct field mutation with address taken
    let mut p = Pair { a: 1, b: 2 };
    let r = &mut p;
    r.swap();
    acc = acc.wrapping_add(p.sum());
    // tuple roundtrip
    for i in 0..1000u64 { acc = acc.wrapping_add(tuple_roundtrip(i, acc)); }
    // Option through a loop
    acc = acc.wrapping_add(opt_roundtrip(Some(12345)));
    acc = acc.wrapping_add(opt_roundtrip(None));
    // RefCell interior mutation between stores
    let cell = RefCell::new(10u64);
    let mut local = 0u64;
    for i in 0..500u64 {
        let cur = *cell.borrow();
        *cell.borrow_mut() = cur.wrapping_add(i);
        local = local.wrapping_add(*cell.borrow());
    }
    acc = acc.wrapping_add(local);
    // array on stack, partial overwrite then read
    let mut arr = [1u64, 2, 3, 4];
    arr[1] = 99;
    arr[3] = arr[0].wrapping_add(arr[1]);
    acc = acc.wrapping_add(arr.iter().copied().fold(0, u64::wrapping_add));
    println!("{}", acc);
}
