fn main() {
    let mut x: i64 = 0;
    let mut i: i64 = 0;
    while i < 4 {
        loop {
            if { break; true } {}
        }
        i = i + 1;
        x = x + i;
    }
    println!("{}", x);
}
