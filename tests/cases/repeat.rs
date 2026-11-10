#[derive(Copy, Clone)]
struct Pair { a: [i64; 3], tag: i64 }

fn sum3(v: [i64; 3]) -> i64 { v[0] + v[1] + v[2] }
fn make() -> [i64; 3] { [9; 3] }

fn main() {
    let i: i64 = 2;
    println!("{}", [10; 3][i as usize]);
    println!("{}", [10, 20, 30][i as usize]);
    println!("{}", sum3([4; 3]));
    let r: [i64; 3] = make();
    println!("{}", r[1]);
    let p: Pair = Pair { a: [7; 3], tag: 1 };
    println!("{}", p.a[2]);
    println!("{}", p.tag);
}
