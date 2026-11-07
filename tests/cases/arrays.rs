fn main() {
    let mut a: [i64; 10] = [0; 10];
    let mut i: i64 = 0;
    while i < 10 {
        a[i as usize] = i * i;
        i += 1;
    }
    let mut s: i64 = 0;
    i = 0;
    while i < 10 {
        println!("{}", a[i as usize]);
        s += a[i as usize];
        i += 1;
    }
    println!("{}", a[0]);
    println!("{}", a[9]);
    println!("{}", s);
    let b: [i64; 5] = [7; 5];
    println!("{}", b[0]);
    println!("{}", b[4]);
    let c: [i64; 3] = [9; 3];
    println!("{}", c[0]);
    println!("{}", c[1]);
    println!("{}", c[2]);
    let mut d: [i64; 4] = [0; 4];
    d[3] = 42;
    d[0] = d[3] * 2;
    println!("{}", d[0]);
    println!("{}", d[3]);
}
