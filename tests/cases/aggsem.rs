#[derive(Copy, Clone)]
struct Inner { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct Outer { inner: Inner, z: i64 }

fn make3(x: i64) -> [i64; 3] { [x, x + 1, x + 2] }

fn main() {
    let mut a: [i64; 2] = [0; 2];
    let b: [i64; 2] = [3; 2];
    a = b;
    println!("{}", a[0]);
    println!("{}", a[1]);

    let c = [7; 2];
    println!("{}", c[0]);
    println!("{}", c[1]);

    let o: Outer = Outer { inner: Inner { x: 1, y: 2 }, z: 3 };
    println!("{}", o.inner.x);
    println!("{}", o.inner.y);
    println!("{}", o.z);

    println!("{}", make3(10)[0]);
    println!("{}", make3(10)[2]);

    let mut p: Inner = Inner { x: 5, y: 6 };
    let q: Inner = Inner { x: 7, y: 8 };
    p = q;
    println!("{}", p.x);
    println!("{}", p.y);
}
