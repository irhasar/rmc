fn take_mut(mut x: i64) -> i64 { let p = &mut x; *p = 9; x }
fn read_ref(x: i64) -> i64 { let p = &x; *p }

fn main() {
    let y: i64 = 1;
    let r = take_mut(y);
    println!("{}", y);
    println!("{}", r);
    let z: i64 = 5;
    println!("{}", read_ref(z));
}
