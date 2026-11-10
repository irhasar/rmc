// A path-qualified *struct* constant (`std::ops::Range::<usize> { start, end }`)
// must not be mistaken for an enum variant: doing so reserved a tag word and
// shifted the fields, so `r.start` read the tag (`0`) instead of `1`. The
// printer spells the constant with its full path, while a construction
// (`Range { start, end }`) uses the bare name, so both spellings must resolve
// to the same layout.
fn main() {
    let r = 1usize..3usize;
    println!("{} {}", r.start, r.end);

    let s = 5i64..9i64;
    println!("{}", s.start * s.end);

    // A user struct written with an explicit path (a module) keeps working.
    let p = geo::Point { x: 3, y: 4 };
    println!("{}", p.x + p.y);

    // An enum-variant constant still carries its tag.
    let o: Option<i32> = Some(7);
    match o {
        Some(v) => println!("{}", v),
        None => println!("none"),
    }
}

mod geo {
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }
}
