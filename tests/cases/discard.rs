fn side(n: i64) -> i64 { println!("{}", n); n + 1 }

struct P { x: i64 }

fn main() {
    let _ = 3 + 4;
    let _ = side(10);
    let _ = side(20) + side(30);

    let _: i64 = side(40);
    let _: i64;
    let _: bool = true;

    let _ = [1, 2, 3];
    let _ = P { x: 7 };
    let _ = if true { side(50) } else { side(60) };

    let x: i64 = 5;
    let _ = x;
    println!("{}", x);

    let mut i: i64 = 0;
    while i < 2 {
        let _ = side(70);
        i += 1;
    }
}
