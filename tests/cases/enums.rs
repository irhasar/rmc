#[derive(Copy, Clone)]
enum Color { Red, Green, Blue }

#[derive(Copy, Clone)]
enum Dir { North, East, South, West }

impl Dir {
    fn dx(&self) -> i64 {
        match self { Dir::East => 1, Dir::West => 0 - 1, _ => 0 }
    }
    fn dy(&self) -> i64 {
        match self { Dir::North => 1, Dir::South => 0 - 1, _ => 0 }
    }
}

#[derive(Copy, Clone)]
struct Point { x: i64, y: i64 }

impl Point {
    fn shift(&self, d: Dir) -> Point {
        Point { x: self.x + d.dx(), y: self.y + d.dy() }
    }
}

fn is_warm(c: Color) -> bool {
    match c { Color::Red => true, _ => false }
}

fn main() {
    println!("{}", Color::Red as i64);
    println!("{}", Color::Green as i64);
    println!("{}", Color::Blue as i64);
    let c = Color::Green;
    println!("{}", is_warm(c) as i64);
    println!("{}", is_warm(Color::Red) as i64);
    let p = Point { x: 0, y: 0 };
    let q = p.shift(Dir::East).shift(Dir::East).shift(Dir::North);
    println!("{}", q.x);
    println!("{}", q.y);
    let mut s = Color::Red;
    let mut n = 0;
    let mut go = 1;
    while go == 1 {
        if s as i64 == 0 { s = Color::Green; }
        else if s as i64 == 1 { s = Color::Blue; }
        else { go = 0; }
        n += 1;
    }
    println!("{}", n);
    println!("{}", s as i64);
}
