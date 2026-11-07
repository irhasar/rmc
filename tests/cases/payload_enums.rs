#[derive(Copy, Clone)]
enum Opt { None, Some(i64) }

#[derive(Copy, Clone)]
enum Shape { Dot, Line(i64, i64), Rect(i64, i64, i64) }

#[derive(Copy, Clone)]
struct Point { x: i64, y: i64 }

#[derive(Copy, Clone)]
enum Node { Leaf(i64), Pair(Point) }

fn get(o: Opt) -> i64 {
    match o { Opt::Some(v) => v, Opt::None => 0 - 1, _ => 0 }
}

impl Opt {
    fn or_else(self, d: i64) -> i64 {
        match self { Opt::Some(v) => v, Opt::None => d }
    }
    fn is_some(&self) -> bool {
        match *self { Opt::Some(v) => true, Opt::None => false }
    }
}

fn area(s: Shape) -> i64 {
    match s {
        Shape::Dot => 0,
        Shape::Line(w, h) => w * h,
        Shape::Rect(w, h, d) => 2 * (w * h + h * d + w * d),
        _ => 0,
    }
}

#[derive(Copy, Clone)]
struct Box2 { v: i64 }

impl Box2 {
    fn wrap(&self) -> Opt {
        // A payload taken from a field: the argument is a memory operand, which
        // a constructor must route through a register.
        Opt::Some(self.v)
    }
    fn empty(&self) -> Opt { Opt::None }
}

fn sum_node(n: Node) -> i64 {
    match n { Node::Leaf(v) => v, Node::Pair(p) => p.x + p.y, _ => 0 }
}

fn main() {
    println!("{}", get(Opt::Some(42)));
    println!("{}", get(Opt::None));
    println!("{}", area(Shape::Dot));
    println!("{}", area(Shape::Line(3, 4)));
    println!("{}", area(Shape::Rect(1, 2, 3)));
    println!("{}", sum_node(Node::Leaf(9)));
    println!("{}", sum_node(Node::Pair(Point { x: 5, y: 6 })));
    let mut acc = 0;
    let mut i = 0;
    while i < 5 {
        let o = Opt::Some(i * 10);
        match o { Opt::Some(v) => { acc += v; } Opt::None => {} }
        i += 1;
    }
    println!("{}", acc);
    let a = Opt::Some(7);
    let b = a;
    println!("{}", get(a) + get(b));
    let bx = Box2 { v: 55 };
    println!("{}", get(bx.wrap()));
    println!("{}", get(bx.empty()));
    println!("{}", Opt::Some(8).or_else(0));
    println!("{}", Opt::None.or_else(50));
    println!("{}", Opt::Some(1).is_some() as i64);
    println!("{}", Opt::None.is_some() as i64);
    let ro: &Opt = &a;
    println!("{}", ro.or_else(0));
    let c = Opt::None;
    match c {
        Opt::Some(v) => { println!("{}", v); }
        other => { println!("{}", get(other) - 100); }
    }
}
