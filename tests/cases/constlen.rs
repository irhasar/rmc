fn main() {
    let b: [i64; 1 | 2] = [1, 2, 3];
    println!("{}", b[0] + b[1] + b[2]);
    let c = [7; 4 & 6];
    println!("{}", c[3]);
    let d = [9; 5 ^ 1];
    println!("{}", d[3]);
    let e = [4; 1 << 3];
    println!("{}", e[7]);
}
