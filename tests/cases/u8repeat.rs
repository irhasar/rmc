// A repeat literal takes the declared element width: `[0; 24]` in a
// `[u8; 24]` binding is a `[u8; 24]`. rmc typed the `0` as `i64` and
// rejected the `let`. The guard array after it catches a store that is wider
// than the element.
fn main() {
    let n: i64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let mut b: [u8; 24] = [0; 24];
    let w: [u16; 3] = [7; 3];
    let guard: [i64; 2] = [n; 2];
    let mut i: usize = 0;
    while i < 24 { b[i] = (i as u8) * 3; i += 1; }
    let mut s: i64 = 0;
    let mut k: usize = 0;
    while k < 24 { s += b[k] as i64; k += 1; }
    println!("{} {} {}", s, w[2] as i64, guard[0] + guard[1]);
}
