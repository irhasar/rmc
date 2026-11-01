use std::env;
fn main() {
    let args: Vec<i64> = env::args()
        .skip(1)
        .map(|s| s.parse::<i64>().unwrap())
        .collect();
    let mut s: i64 = 0;
    for v in &args {
        println!("{}", v);
        s += v;
    }
    println!("{}", s);
}
