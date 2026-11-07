fn tick(x: &mut i64) -> bool {
    *x = *x + 1;
    *x < 4
}

fn next(x: &mut i64) -> i64 {
    *x = *x + 1;
    0
}

fn main() {
    let mut x: i64 = 0;
    let mut sum: i64 = 0;
    while tick(&mut x) {
        sum = sum + x * x;
    }
    println!("{}", sum);

    let mut y: i64 = 0;
    let mut a = [0i64; 4];
    let mut j: i64 = 0;
    let mut s3: i64 = 0;
    while j < 3 {
        s3 = s3 + y * y;
        a[next(&mut y) as usize] = 0;
        j = j + 1;
    }
    println!("{}", s3);
    println!("{}", y);
}
