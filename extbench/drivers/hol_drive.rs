// holidays-rs: holiday/working-day queries over many countries and years.
use holidays_rs::{Calendar, Country, Date, Direction};

fn main() {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(40_000);
    let mut acc: u64 = 0;
    let mut i: u64 = 0;
    while i < n {
        for &c in Country::ALL {
            let cal = Calendar::new(c);
            let year = 1900 + (i % 200) as i32;
            for (d, name) in cal.holidays_in(year) {
                acc = acc.wrapping_add((d.year as u64) ^ ((d.month as u64) << 8) ^ ((d.day as u64) << 16) ^ (name.len() as u64));
            }
            if let Some(ref0) = Date::new(2000, 6, 15) {
                if let Some((d, _)) = cal.closest_holiday(ref0, Direction::Forward) {
                    acc = acc.wrapping_add(d.month as u64);
                }
            }
        }
        i += 1;
    }
    println!("{}", acc);
}
