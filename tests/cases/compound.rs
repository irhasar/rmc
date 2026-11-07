fn main() {
    let mut x: i64 = 100;
    x += 5;
    println!("{}", x);
    x -= 20;
    println!("{}", x);
    x *= 3;
    println!("{}", x);
    x /= 4;
    println!("{}", x);
    x %= 7;
    println!("{}", x);
    let mut arr: [i64; 3] = [1, 2, 3];
    arr[1] += 10;
    println!("{}", arr[0] + arr[1] + arr[2]);
}
