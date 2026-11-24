// Open Location Code: encode -> decode -> shorten -> recover over a grid.
use openlocationcode::{decode, encode, recover_nearest, shorten};

fn main() {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let mut acc: u64 = 0;
    let mut i: u64 = 0;
    while i < n {
        // pseudo-random lat/lng over the valid range
        let r = i.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let lat = ((r >> 11) % 180_000_000) as f64 / 1_000_000.0 - 90.0;
        let lng = ((r >> 33) % 360_000_000) as f64 / 1_000_000.0 - 180.0;
        if let Ok(code) = encode(lat, lng, 10) {
            if let Ok(area) = decode(&code) {
                let c = ((area.latitude_center * 1e6) as i64) ^ ((area.longitude_center * 1e6) as i64);
                acc = acc.wrapping_add(c as u64);
                if let Ok(s) = shorten(&code, lat, lng) {
                    if let Ok(full) = recover_nearest(&s, lat, lng) {
                        acc = acc.wrapping_add(full.len() as u64);
                    }
                }
            }
        }
        i += 1;
    }
    println!("{}", acc);
}
