// A tuple struct (`struct P(T, T);`) stores its fields under the positional
// names `"0"`, `"1"`, ..., so it lays out exactly like a braced struct with the
// same field types. Only the surface syntax differs: `P(a, b)` constructs it,
// `p.0` reads a field, and `P(x, y)` destructures it.
struct Point(i64, i64);
struct Meters(i64);
struct Wrap(Point);

fn dist2(p: Point) -> i64 {
    let Point(x, y) = p;
    x * x + y * y
}

fn sum(Pair(a, b): Pair) -> i64 {
    a + b
}

struct Pair(i64, i64);

fn main() {
    let p = Point(3, 4);
    println!("{} {}", p.0, p.1);
    println!("{}", dist2(p));

    // A one-field tuple struct, and a tuple struct nested inside another.
    let m = Meters(42);
    println!("{}", m.0);
    let w = Wrap(Point(1, 2));
    println!("{} {}", w.0 .0, w.0 .1);

    // Nested destructuring.
    let Wrap(Point(a, b)) = w;
    println!("{} {}", a, b);

    // A `let` and a `match` both bind the fields positionally.
    let Point(c, d) = Point(7, 8);
    println!("{} {}", c, d);
    match Point(5, 6) {
        Point(x, y) => println!("{} {}", x, y),
    }

    // A field is a place: it can be read, written and borrowed.
    let mut q = Point(1, 2);
    q.0 = 10;
    q.1 = q.0 + q.1;
    println!("{} {}", q.0, q.1);
    let r = &q;
    println!("{}", r.0);

    // `_` ignores a field, and `mut` may attach to an individual binding.
    let Point(_, y) = Point(9, 8);
    println!("{}", y);
    let Point(mut z, _) = Point(1, 2);
    z += 100;
    println!("{}", z);

    // A parameter pattern destructures without naming the field positions.
    println!("{}", sum(Pair(10, 20)));

    // Tuple structs are ordinary array elements and tuple members.
    let ps = [Point(1, 1), Point(2, 2)];
    println!("{} {}", ps[0].0, ps[1].1);
    let t = (Point(5, 6), 7);
    println!("{} {} {}", t.0 .0, t.0 .1, t.1);
    let (Point(e, f), g) = t;
    println!("{} {} {}", e, f, g);

    for Point(x, y) in ps {
        println!("[{} {}]", x, y);
    }
}

// A unit struct is a zero-sized value named by its type: `U` is its value.
struct Unit;

fn make_unit() -> Unit {
    Unit
}

fn unit_ok() {
    let u = Unit;
    let Unit = u;
    let Unit = make_unit();
    println!("unit ok");
}
