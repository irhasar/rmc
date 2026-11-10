// A struct-typed field of an aggregate constant must be laid out into the
// enclosing region, not stored as a pointer to a separately materialised
// value. The frame optimiser's dead-store passes only see direct `(%rbp)`
// stores, so an interior field written as `movq $v, slot(%rbp)` looks unread
// and is deleted: `B { a: A { x: 5 }, y: 6 }` printed `5 0`.
struct Inner {
    x: i64,
    y: i64,
}

struct Outer {
    head: Inner,
    tail: i64,
}

fn main() {
    let o = Outer {
        head: Inner { x: 5, y: -6 },
        tail: 7,
    };
    println!("{} {} {}", o.head.x, o.head.y, o.tail);

    // A struct constant nested one level deeper still.
    let nested = Outer {
        head: Inner { x: -1, y: 2 },
        tail: -3,
    };
    println!("{} {} {}", nested.head.x, nested.head.y, nested.tail);

    // An array of structs, built from a struct constant.
    let arr = [Inner { x: 10, y: 20 }, Inner { x: -30, y: 40 }];
    println!("{} {} {} {}", arr[0].x, arr[0].y, arr[1].x, arr[1].y);

    // A tuple holding a struct constant.
    let t = (Inner { x: -7, y: 8 }, 9);
    println!("{} {} {}", t.0.x, t.0.y, t.1);
}
