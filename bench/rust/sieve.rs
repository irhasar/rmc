use std::env;
fn main() {
    let limit: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(200000);
    let limit = std::hint::black_box(limit);
    let mut flags = [1i64; 200000];
    let mut count: i64 = 0;
    let mut i: i64 = 2;
    while i < limit {
        if flags[i as usize] == 1 {
            count += 1;
            let mut j: i64 = i * i;
            while j < limit {
                flags[j as usize] = 0;
                j += i;
            }
        }
        i += 1;
    }
    println!("{}", count);
}
