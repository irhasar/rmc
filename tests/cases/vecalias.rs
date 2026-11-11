// A `Vec` element named through a `type` alias resolves to the alias target's
// byte width, so `Vec<Byte>` with `type Byte = u8` keeps a one-byte stride, and
// a chain of aliases is followed too.
type Byte = u8;
type B2 = Byte;
type Word = i64;

fn main() {
    let mut v: Vec<Byte> = vec![0u8; 16];
    let p = v.as_mut_ptr();
    let q = unsafe { p.add(8) };
    println!("{}", q as i64 - p as i64);

    let mut a: Vec<B2> = vec![0u8; 16];
    let p = a.as_mut_ptr();
    let q = unsafe { p.add(5) };
    println!("{}", q as i64 - p as i64);

    let mut w: Vec<Word> = vec![0i64; 4];
    let p = w.as_mut_ptr();
    let q = unsafe { p.add(3) };
    println!("{}", q as i64 - p as i64);
}
