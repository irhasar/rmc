use std::env;
fn main() {
    let size: i64 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(400);
    let size = std::hint::black_box(size);
    let mut acc: i64 = 0;
    let mut y: i64 = 0;
    while y < size {
        let mut x: i64 = 0;
        while x < size {
            let mut zr: i64 = 0;
            let mut zi: i64 = 0;
            let cr: i64 = x - 200;
            let ci: i64 = y - 200;
            let mut i: i64 = 0;
            while i < 50 {
                let zr2: i64 = (zr * zr) / 1000;
                let zi2: i64 = (zi * zi) / 1000;
                if zr2 + zi2 > 4000 { break; }
                let nzi: i64 = (2 * zr * zi) / 1000 + ci;
                zr = zr2 - zi2 + cr;
                zi = nzi;
                i += 1;
            }
            acc += i;
            x += 1;
        }
        y += 1;
    }
    println!("{}", acc);
}
