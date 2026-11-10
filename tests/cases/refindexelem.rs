#[derive(Copy, Clone)]
enum E { A, B }

fn main() {
    let a = [false, true, true, false];
    let r: &[bool; 4] = &a;
    let b: bool = !r[0];
    println!("{}", b as i64);
    println!("{}", !r[1] as i64);
    println!("{}", !r[2] as i64);
    println!("{}", !r[3] as i64);

    let e = [E::A, E::B];
    let er: &[E; 2] = &e;
    println!("{}", (er[0] as i64 == 0) as i64);
    println!("{}", (er[1] as i64 == 1) as i64);
}
