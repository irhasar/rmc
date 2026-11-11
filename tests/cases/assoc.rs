struct Point { x: i64, y: i64 }

impl Point {
    fn new(x: i64, y: i64) -> Point { Point { x, y } }
    fn origin() -> Point { Point { x: 0, y: 0 } }
    fn twice(n: i64) -> i64 { n * 2 }

    fn sum(&self) -> i64 { self.x + self.y }
    fn shifted(&self, d: i64) -> Point { Point::new(self.x + d, self.y + d) }
}

enum Tag { Off, On }

impl Tag {
    fn from_flag(b: bool) -> Tag { if b { Tag::On } else { Tag::Off } }
    fn is_on(self) -> bool { self as i64 == 1 }
}

enum Opt { None, Some(i64) }

impl Opt {
    fn wrap(v: i64) -> Opt { Opt::Some(v) }
}

fn main() {
    let p = Point::new(3, 4);
    println!("{}", p.sum());
    println!("{}", Point::new(1, 2).sum());
    println!("{}", Point::origin().sum());
    println!("{}", Point::twice(21));
    println!("{}", p.shifted(10).sum());

    println!("{}", Tag::from_flag(true).is_on() as i64);
    println!("{}", Tag::from_flag(false).is_on() as i64);
    match Opt::wrap(9) {
        Opt::Some(v) => println!("{}", v),
        Opt::None => println!("{}", 0),
    }

    let x: i64 = 5;
    let r: &i64 = &x;
    println!("{}", *r as i64);
    println!("{}", -x as i64);
    let b: bool = true;
    println!("{}", b as i64 * 2);
}
