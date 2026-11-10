#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }

fn next(c: i64) -> i64 { println!("{}", c); 1 }

fn main() {
    let mut a: [i64; 3] = [0; 3];
    a[next(99) as usize] += 5;
    println!("{}", a[0]);
    println!("{}", a[1]);
    println!("{}", a[2]);

    let s = P { y: { println!("{}", 1); 2 }, x: { println!("{}", 2); 3 } };
    println!("{}", s.x);
    println!("{}", s.y);
}
