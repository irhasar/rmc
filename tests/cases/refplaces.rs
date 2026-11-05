// The Rust twin of `refplaces.rms`. A plain Rust subset, so it compiles with
// rustc unchanged.
#[derive(Copy, Clone)]
struct Inner { a: i64, b: i64 }
#[derive(Copy, Clone)]
struct Outer { s: i64, mid: Inner, arr: [i64; 3] }

fn inc(r: &mut i64) { *r += 1; }
fn addto(r: &mut i64, v: i64) { *r += v; }
fn sumfields(r: &Outer) -> i64 { r.s + r.mid.a + r.mid.b + r.arr[0] }
fn setmid(r: &mut Outer, v: i64) { r.mid.a += v; }

fn main() {
    let mut x: i64 = 10;
    inc(&mut x);
    addto(&mut x, 5);
    println!("{}", x);

    let mut o: Outer = Outer { s: 1, mid: Inner { a: 2, b: 3 }, arr: [4, 5, 6] };
    inc(&mut o.s);
    inc(&mut o.mid.a);
    addto(&mut o.mid.b, 10);
    println!("{}", o.s + o.mid.a + o.mid.b);
    println!("{}", sumfields(&o));
    setmid(&mut o, 7);
    println!("{}", o.mid.a);

    let mut a: [i64; 3] = [1, 2, 3];
    inc(&mut a[0]);
    addto(&mut a[2], 10);
    println!("{}", a[0] + a[1] + a[2]);

    let mut m: [[i64; 2]; 2] = [[1, 2], [3, 4]];
    inc(&mut m[1][1]);
    inc(&mut m[0][0]);
    println!("{}", m[0][0] + m[0][1] + m[1][0] + m[1][1]);

    let o2: Outer = Outer { s: 5, mid: Inner { a: 6, b: 7 }, arr: [8, 9, 10] };
    println!("{}", sumfields(&o2));

    let mut acc: i64 = 100;
    let rl: &mut i64 = &mut acc;
    *rl += 1;
    println!("{}", acc);
}
