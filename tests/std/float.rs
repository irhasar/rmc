// Floating point on the MIR path: `f64`/`f32` arithmetic, comparison, and the
// int/float casts. The values are printed as bit patterns via `to_bits`/`{:x}`
// rather than with `{}`, because rmc's runtime renders a float's shortest
// decimal form only after this was written; the bit pattern pins the exact
// IEEE-754 result, which is what the arithmetic must reproduce. Comparisons are
// printed as `bool`, so the NaN cases (`!=` true, everything else false) are
// checked too.

fn main() {
    let a = 1.5f64;
    let b = 2.25f64;
    println!("{:x}", (a + b).to_bits());
    println!("{:x}", (a - b).to_bits());
    println!("{:x}", (a * b).to_bits());
    println!("{:x}", (a / b).to_bits());

    println!("{} {} {} {}", a < b, a <= b, a > b, a >= b);
    println!("{} {} {} {}", a == a, a == b, a != a, a != b);

    let nan = f64::NAN;
    println!("{} {} {} {} {} {}", nan == nan, nan != nan, nan < a, nan > a, nan <= a, nan >= a);

    // Float -> int truncates toward zero; int -> float is exact for small
    // magnitudes. A negative value checks the sign, not just the magnitude.
    let i: i64 = (a * 4.0) as i64;
    let j: i64 = (-b * 2.0) as i64;
    println!("{} {}", i, j);
    println!("{:x}", (7i64 as f64).to_bits());
    println!("{:x}", (-7i64 as f64).to_bits());

    // `f32` arithmetic operates on the low 32 bits; widening to `f64` must
    // reproduce the single-precision value exactly.
    let x = 0.1f32;
    let y = 3.0f32;
    println!("{:x}", (x * y).to_bits());
    println!("{:x}", (x as f64).to_bits());
    let w = a as f32;
    println!("{:x}", (w as f64).to_bits());

    // A float in an aggregate field and a float argument through a call.
    fn add(x: f64, y: f64) -> f64 {
        x + y
    }
    println!("{:x}", add(a, b).to_bits());
    let arr = [a, b, 4.0f64];
    println!("{:x}", (arr[0] + arr[2]).to_bits());
}
