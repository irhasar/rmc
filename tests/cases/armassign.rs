fn classify(x: i64) -> i64 {
    match x {
        0 => return 100,
        _ => return x * 2,
    }
}

fn main() {
    let mut acc: i64 = 0;
    let mut x: i64 = 0;
    while x < 12 {
        match x {
            0 => acc = acc + 1,
            1 | 2 => acc += 10,
            3..=5 => acc -= 1,
            _ => {}
        }
        if let 6 | 7 = x { acc = acc + 100; }
        x += 1;
    }
    println!("{}", acc);

    let mut i: i64 = 0;
    let mut s: i64 = 0;
    while i < 10 {
        match i {
            4 => break,
            _ => s = s + i,
        }
        i = i + 1;
    }
    println!("{}", s + classify(3));

    let mut y: i64 = 0;
    while let 0..=4 = y {
        y = y + 1;
    }
    println!("{}", y);
}
