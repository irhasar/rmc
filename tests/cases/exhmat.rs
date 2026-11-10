// Rust twin of exhmat.rms.
#[derive(Copy, Clone)]
enum Color { Red, Green, Blue }

#[derive(Copy, Clone)]
enum Opt { None, Some(i64) }

fn name(c: Color) -> i64 {
    match c {
        Color::Red => 10,
        Color::Green => 20,
        Color::Blue => 30,
    }
}

fn get(o: Opt, d: i64) -> i64 {
    match o {
        Opt::Some(v) => v,
        Opt::None => d,
    }
}

fn main() {
    println!("{}", name(Color::Red));
    println!("{}", name(Color::Green));
    println!("{}", name(Color::Blue));

    let c: Color = Color::Blue;
    let t: i64 = match c {
        Color::Blue => 3,
        Color::Red => 1,
        Color::Green => 2,
    };
    println!("{}", t);

    let o: Opt = Opt::Some(7);
    println!("{}", get(o, 0));
    println!("{}", get(Opt::None, 99));

    let r: &Opt = &o;
    let via_ref: i64 = match *r {
        Opt::Some(v) => v,
        Opt::None => 0 - 1,
    };
    println!("{}", via_ref);

    let mut acc: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        let col: Color = match i % 3 {
            0 => Color::Red,
            1 => Color::Green,
            _ => Color::Blue,
        };
        acc = acc + match col {
            Color::Red => 1,
            Color::Green => 2,
            Color::Blue => 4,
        };
        i = i + 1;
    }
    println!("{}", acc);

    let nested: i64 = match c {
        Color::Red => 100,
        Color::Green => 200,
        Color::Blue => match o {
            Opt::Some(v) => v,
            Opt::None => 0,
        },
    };
    println!("{}", nested);
}
