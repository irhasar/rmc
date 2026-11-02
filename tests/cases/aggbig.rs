#[derive(Copy, Clone)]
struct Big {
    a: i64, b: i64, c: i64, d: i64, e: i64,
    f: i64, g: i64, h: i64, i: i64, j: i64,
}

fn bsum(v: Big) -> i64 {
    v.a + v.b + v.c + v.d + v.e + v.f + v.g + v.h + v.i + v.j
}

fn bmake(x: i64) -> Big {
    Big {
        a: x, b: x + 1, c: x + 2, d: x + 3, e: x + 4,
        f: x + 5, g: x + 6, h: x + 7, i: x + 8, j: x + 9,
    }
}

fn mix(s: i64, v: Big, t: i64) -> i64 {
    s * 1000 + bsum(v) + t
}

fn main() {
    let b = bmake(100);
    println!("{}", bsum(b));
    println!("{}", b.a);
    println!("{}", b.j);
    println!("{}", mix(7, b, 9));
    println!("{}", bsum(bmake(0)));
    let mut c = b;
    c.a = 999;
    println!("{}", c.a);
    println!("{}", b.a);
    println!("{}", bsum(c));
}
