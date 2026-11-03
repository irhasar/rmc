fn main() {
    let mut i: i64 = 0;
    while i < 3 {
        i = i + 1;
    }
    println!("{}", i);

    let mut j: i64 = 0;
    while j < 5 {
        j = j + 1;
    }
    println!("{}", j);

    let mut s: i64 = 0;
    for _ in 0..4 {
        s = s + 1;
    }
    println!("{}", s);

    let mut n: i64 = 0;
    loop {
        n = n + 1;
        if n == 3 {
            break;
        }
    }
    println!("{}", n);
}
