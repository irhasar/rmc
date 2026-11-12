fn main() {
    for i in (0..5).rev() { print!("{} ", i); }
    println!();
    for i in (1..=4).rev() { print!("{} ", i); }
    println!();
    for i in (0..6).rev() { if i == 3 { continue; } print!("{} ", i); }
    println!();
    for i in (0..9).rev() { if i == 4 { break; } print!("{} ", i); }
    println!();
    let a = 2;
    let b = 6;
    for i in (a..b).rev() { print!("{} ", i); }
    println!();
    for i in (-3..0).rev() { print!("{} ", i); }
    println!();
    for i in (3..3).rev() { print!("{} ", i); }
    println!("empty");
    let mut s = 0;
    for _ in (0..4).rev() { s += 1; }
    println!("{}", s);
    for i in (0..3).rev() {
        for j in (0..3).rev() { print!("{}{} ", i, j); }
    }
    println!();
    'a: for i in (0..5).rev() {
        for j in 0..5 { if j == 2 { continue 'a; } }
        print!("{} ", i);
    }
    println!("labelled");
}
