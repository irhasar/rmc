fn main() {
    let a: [i64; 4] = [10, 20, 30, 40];
    let mut sum: i64 = 0;
    for i in 0..4 {
        sum += a[i];
    }
    println!("{}", sum);
    let b: [i64; 3] = [5; 3];
    println!("{}", b[0] + b[1] + b[2]);
}
