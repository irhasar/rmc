fn main() {
    let mut a: [i64; 4] = [0; 4];
    let mut i: i64 = 0;
    while i < 4 {
        a[i as usize] = i * i + 1;
        i = i + 1;
    }
    let mut s: i64 = 0;
    i = 0;
    while i < 4 {
        s = s + a[i as usize];
        i = i + 1;
    }
    println!("{}", s);
    println!("{}", a[0]);
    println!("{}", a[3]);
    let v: i64 = 2;
    println!("{}", a[v as usize]);
}
