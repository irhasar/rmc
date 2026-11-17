// autovectorizable loops + numeric kernels
fn main() {
    let n = 4_000_000usize;
    let mut a = vec![0f32; n];
    let mut b = vec![0f32; n];
    for i in 0..n { a[i] = (i as f32).sin(); b[i] = (i as f32).cos(); }
    let mut s = 0f32;
    for i in 0..n { s += a[i] * b[i] + (a[i] - b[i]).abs(); }
    let mut c = vec![0u32; n];
    for i in 0..n { c[i] = (a[i].to_bits() ^ b[i].to_bits()).wrapping_mul(2654435761); }
    let t: u32 = c.iter().fold(0, |x, y| x ^ y);
    println!("{} {}", s, t);
}
