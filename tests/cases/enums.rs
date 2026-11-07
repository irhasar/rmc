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
    let dirs: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];
    let mut i: i64 = 0;
    let mut sum: i64 = 0;
    while i < 4 {
        sum += dirs[i as usize] as i64;
        i += 1;
    }
    println!("{}", sum);
    let t: i64 = match dirs[3] {
        Dir::West => 99,
        Dir::North => 11,
        Dir::East => 22,
        Dir::South => 33,
    };
    println!("{}", t);
    let mut rep: [Color; 3] = [Color::Blue; 3];
    rep[1] = Color::Red;
    println!("{}", rep[0] as i64);
    println!("{}", rep[1] as i64);
    println!("{}", rep[2] as i64);
}
