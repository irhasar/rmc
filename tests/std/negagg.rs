// A negative literal inside an aggregate is stored with an unsuffixed
// `mov $imm, mem`, which the assembler encodes as a 32-bit store: `$-3`
// became `0xFFFFFFFD` (4294967293), a silent wrong answer. The fix uses
// `movq`/a scratch register for every full-word store.
fn main() {
    let r = -3i64..3i64;
    println!("{} {}", r.start, r.end);

    let a = [-1i64, -2, -3];
    println!("{} {} {}", a[0], a[1], a[2]);

    let repeated = [-7i64; 3];
    println!("{}", repeated[2]);

    let t = (-3i64, 5i64);
    println!("{} {}", t.0, t.1);

    let p = Point { x: -3, y: -4 };
    println!("{} {}", p.x, p.y);

    // A positive field next to a negative one must not be shifted.
    let q = Point { x: 8, y: -9 };
    println!("{} {}", q.x, q.y);
}

struct Point {
    x: i64,
    y: i64,
}
