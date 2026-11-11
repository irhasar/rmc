#[derive(Copy, Clone)]
enum Opt { None, Some(i64) }

fn sign(x: i64) -> i64 {
    match x {
        n if n > 0 => 1,
        n if n < 0 => -1,
        _ => 0,
    }
}

fn describe(o: Opt) -> i64 {
    match o {
        Opt::Some(v) if v > 10 => 100 + v,
        Opt::Some(v) => v,
        Opt::None => -1,
    }
}

fn main() {
    println!("{}", sign(5));
    println!("{}", sign(-5));
    println!("{}", sign(0));

    let x: i64 = 3;
    println!("{}", match x { 3 if x > 1 => 10, _ => 20 });
    println!("{}", match x { 1 | 2 | 3 if x == 3 => 30, _ => 40 });

    println!("{}", match x { _ if x > 10 => 1, _ => 2 });

    let b: bool = true;
    println!("{}", match b { true if b => 7, _ => 9 });

    println!("{}", describe(Opt::Some(5)));
    println!("{}", describe(Opt::Some(50)));
    println!("{}", describe(Opt::None));

    let mut calls: i64 = 0;
    let r: i64 = match x {
        3 if { calls = calls + 1; x > 5 } => 1,
        3 => 2,
        _ => 3,
    };
    println!("{}", r);
    println!("{}", calls);
}
