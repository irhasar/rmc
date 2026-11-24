// Exercises image's pure-Rust pixel loops: blur, resize, and a per-pixel sum.
// No codecs are used, so the hot code is all in image itself.
use image::{ImageBuffer, Rgb};

fn main() {
    let n: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2048);
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(n, n, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8])
    });

    // Gaussian-ish blur: many passes over the buffer, all in Rust.
    for _ in 0..6 {
        img = image::imageops::blur(&img, 2.5);
    }

    // Resize down then back up.
    let small = image::imageops::resize(&img, n / 2, n / 2, image::imageops::FilterType::Lanczos3);
    let big = image::imageops::resize(&small, n, n, image::imageops::FilterType::CatmullRom);

    // Per-pixel integer work, so the loop body is the measured code.
    let mut acc: u64 = 0;
    for p in big.pixels() {
        acc = acc.wrapping_add(p[0] as u64 * 3 + p[1] as u64 * 5 + p[2] as u64 * 7);
    }
    println!("{}", acc);
}
