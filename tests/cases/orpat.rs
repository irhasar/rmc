#[derive(Copy, Clone)]
enum Color { Red, Green, Blue }

fn classify(c: Color) -> i64 {
    match c {
        Color::Red | Color::Green => 1,
        Color::Blue => 2,
    }
}

fn main() {
    let x: i64 = 7;
    println!("{}", match x { 1 | 3 | 7 | 9 => 10, _ => 0 });
    println!("{}", match x { 0 | 1 => 5, _ => 9 });

    let c: Color = Color::Green;
    println!("{}", classify(c));
    println!("{}", classify(Color::Blue));

    let all: i64 = match c {
        Color::Red | Color::Green | Color::Blue => 42,
    };
    println!("{}", all);

    let b: bool = false;
    println!("{}", match b { true | false => 3 });

    println!("{}", match x { 1 | _ => 8 });

    let n: i64 = -1;
    println!("{}", match n { -1 | -2 => 5, _ => 0 });

    if let Color::Red | Color::Green = c {
        println!("{}", 100);
    } else {
        println!("{}", 200);
    }
}
