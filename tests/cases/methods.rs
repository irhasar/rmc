struct Point { x: i64, y: i64 }
struct Counter { n: i64 }

impl Point {
    fn sum(&self) -> i64 { self.x + self.y }
    fn moved(&self, dx: i64, dy: i64) -> Point { Point { x: self.x + dx, y: self.y + dy } }
    fn same(&self, o: Point) -> bool { self.x == o.x && self.y == o.y }
    fn scale(&self, k: i64) -> Self { Self { x: self.x * k, y: self.y * k } }
    fn norm1(&self) -> i64 { if self.x < 0 { 0 - self.x + self.y } else { self.x + self.y } }
}

impl Counter {
    fn factorial(&self) -> i64 {
        let mut r = 1; let mut i = 1;
        while i <= self.n { r *= i; i += 1; }
        r
    }
    fn is_big(&self) -> bool { self.n > 3 }
}

fn main() {
    let p = Point { x: 3, y: 4 };
    println!("{}", p.sum());
    let q = p.moved(1, 2);
    println!("{}", q.sum());
    println!("{}", p.same(Point { x: 3, y: 4 }) as i64);
    println!("{}", p.same(q) as i64);
    let s = p.scale(2);
    println!("{}", s.x);
    println!("{}", s.y);
    println!("{}", p.scale(2).scale(3).sum());
    println!("{}", Point { x: 10, y: 20 }.sum());
    let c = Counter { n: 5 };
    println!("{}", c.factorial());
    println!("{}", c.is_big() as i64);
    println!("{}", Point { x: 0 - 5, y: 2 }.norm1());
}
