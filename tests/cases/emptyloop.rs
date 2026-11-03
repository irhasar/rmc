fn next(p: &mut i64) -> i64 {
    *p += 1;
    *p
}

fn main() {
    let mut i: i64 = 0;
    while next(&mut i) < 3 {}
    println!("{}", i);

    let mut s: i64 = 0;
    for _k in 0..4 {}
    println!("{}", s);

    let mut j: i64 = 0;
    while j < 5 {
        j += 1;
    }
    println!("{}", j);

    let mut n: i64 = 0;
    loop {
        n += 1;
        if n == 3 {
            break;
        }
    }
    println!("{}", n);
}
