#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }
#[derive(Copy, Clone)]
struct I { u: i64 }
#[derive(Copy, Clone)]
struct S { a: [i64; 2], b: I }

fn sum_struct(P { x, y }: P) -> i64 {
    x * 10 + y
}

fn sum_array([a, b]: [i64; 2]) -> i64 {
    a * 100 + b
}

fn nested(S { a: [p, q], b: I { u } }: S) -> i64 {
    p + q + u
}

fn renamed(P { x: v, mut y }: P) -> i64 {
    y = y + 1;
    v + y
}

fn rest(P { x, .. }: P) -> i64 {
    x
}

fn mixed(k: i64, P { x, y }: P, m: i64) -> i64 {
    k + x + y + m
}

impl P {
    fn add(self, P { x, y }: P) -> i64 {
        self.x + x + y
    }
}

fn main() {
    println!("{}", sum_struct(P { x: 1, y: 2 }));
    println!("{}", sum_array([3, 4]));
    println!("{}", nested(S { a: [5, 6], b: I { u: 7 } }));
    println!("{}", renamed(P { x: 8, y: 9 }));
    println!("{}", rest(P { x: 10, y: 11 }));
    println!("{}", P { x: 1, y: 2 }.add(P { x: 3, y: 4 }));

    println!("{}", mixed(100, P { x: 12, y: 13 }, 200));
}
