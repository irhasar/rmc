struct S { n: i64 }

fn sum(a: i64, b: i64) -> i64 { a + b }
fn set(p: &mut i64) -> i64 { *p = 10; 0 }

fn main() {
    let mut x: i64 = 1;
    println!("{}", sum(x, set(&mut x)));

    let mut y: i64 = 1;
    let mut a: [i64; 1] = [0; 1];
    a[{ y = 2; 0 }] = y;
    println!("{}", a[0]);

    let arr: [i64; 3] = [7, 8, 9];
    let r: &[i64; 3] = &arr;
    let i: i64 = 2;
    println!("{}", r[i as usize]);
}
