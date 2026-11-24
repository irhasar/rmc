// stdnum-rs: checksum/identifier validation over a stream of generated inputs.
use stdnum_rs::{damm, ean, iban, isbn, luhn, verhoeff};

fn main() {
    let n: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let mut acc: u64 = 0;
    let mut i: u64 = 0;
    while i < n {
        let r = i.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let digits = format!("{:012}", r % 1_000_000_000_000);
        if let Ok(v) = ean::Ean::parse(&digits) {
            acc = acc.wrapping_add(v.as_str().len() as u64);
        }
        if let Ok(v) = luhn::Luhn::parse(&digits) {
            acc = acc.wrapping_add(v.check_digit() as u64);
        }
        if let Ok(v) = damm::Damm::parse(&digits) {
            acc = acc.wrapping_add(v.check_digit() as u64);
        }
        if let Ok(v) = verhoeff::Verhoeff::parse(&digits) {
            acc = acc.wrapping_add(v.as_str().len() as u64);
        }
        let iban_str = format!("GB{:02}ABCD{:014}", r % 100, r % 100_000_000_000_000);
        if let Ok(v) = iban::Iban::parse(&iban_str) {
            acc = acc.wrapping_add(v.country_code().len() as u64);
        }
        if let Ok(v) = isbn::Isbn::parse("978-3-16-148410-0") {
            acc = acc.wrapping_add(v.is_isbn13() as u64);
        }
        i += 1;
    }
    println!("{}", acc);
}
