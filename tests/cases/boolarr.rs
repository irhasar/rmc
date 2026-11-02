fn count(t: [bool; 5]) -> i64 {
    let mut n = 0;
    for i in 0..5 { if t[i] { n += 1; } }
    n
}

fn c_or(x: bool) -> bool { x }

fn main() {
    let a: [bool; 5] = [true, false, true, true, false];
    println!("{}", count(a));
    println!("{}", count([c_or(a[0]), a[1], true, false, true]));
    let mut b: [bool; 3] = [false; 3];
    b[1] = true;
    println!("{}", b[0] as i64);
    println!("{}", b[1] as i64);
    println!("{}", b[2] as i64);
    let d = [a[0], b[1]];
    println!("{}", d[0] as i64);
    println!("{}", d[1] as i64);
}
