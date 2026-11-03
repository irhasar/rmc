struct S { x: i64, y: i64 }

fn main() {
    let s = S { x: 42, y: 7 };
    let p = &s;
    let q = &p;
    println!("{}", (*q).y);
    println!("{}", (*q).x);

    let mut a: i64 = 4294967296;
    let r = &mut a;
    *r = 42;
    println!("{}", a);

    let mut b = S { x: 1, y: 2 };
    let mut pb = &mut b;
    let qb = &mut pb;
    (*qb).x = 9;
    (*qb).y = 8;
    println!("{}", b.x);
    println!("{}", b.y);
}
