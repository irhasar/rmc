fn main() {
    let mut x: i64 = 0;
    let mut i: i64 = 0;
    'outer: loop {
        i = i + 1;
        if i > 5 { break; }
        let mut j: i64 = 0;
        loop {
            j = j + 1;
            x = x + 1;
            if j >= 2 { continue 'outer; }
        }
    }
    println!("{}", x);
}
