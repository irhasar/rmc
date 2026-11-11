// The code is a plain Rust subset, so this
// compiles with rustc unchanged.
#[derive(Copy, Clone)]
struct Point { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct Big { a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64 }

impl Point {
    fn mag2(&self) -> i64 { self.x * self.x + self.y * self.y }
    fn bumped(self, d: i64) -> i64 { self.x + d }
}

impl Big {
    fn total(&self) -> i64 { self.a + self.b + self.c + self.d + self.e + self.f + self.g + self.h }
}

fn incr(p: &i64) -> i64 { *p + 1 }

fn sum_arr(a: &[i64; 4]) -> i64 {
    let mut i = 0;
    let mut s = 0;
    while i < 4 { s += a[i]; i += 1; }
    s
}

fn mag_of(p: &Point) -> i64 { p.mag2() + p.x }

fn identity(p: &i64) -> &i64 { p }

fn main() {
    let v: i64 = 41;
    let r: &i64 = &v;
    println!("{}", *r);
    println!("{}", incr(r));

    let mut m: i64 = 5;
    let mr: &i64 = &m;
    println!("{}", *mr);
    m = 9;
    let mr2: &i64 = &m;
    println!("{}", *mr2);

    let arr: [i64; 4] = [10, 20, 30, 40];
    let ar: &[i64; 4] = &arr;
    println!("{}", sum_arr(ar));
    println!("{}", ar[2]);

    let p = Point { x: 3, y: 4 };
    let pr: &Point = &p;
    println!("{}", pr.mag2());
    println!("{}", pr.x);
    println!("{}", mag_of(pr));

    println!("{}", p.bumped(10));

    let r2: &&i64 = &r;
    println!("{}", **r2);
    // A field through a double reference: the inner pointer must be loaded, not
    // mistaken for a register operand (it is not one).
    let ppr: &&Point = &pr;
    println!("{}", (**ppr).x);
    let r3: &i64 = identity(r);
    println!("{}", *r3);

    let fx: &i64 = &p.y;
    println!("{}", *fx);
    let ae: &i64 = &arr[1];
    println!("{}", *ae);

    let big = Big { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8 };
    let br: &Big = &big;
    println!("{}", br.total());
    println!("{}", big.total());
}
