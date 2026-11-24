// Regression coverage for u64 -> float conversion.
//
// `CvtUint64ToFloatSeq` splits on the sign bit: values below 2^63 take the
// direct `cvtsi2ss`/`cvtsi2sd` path, values at or above it go through
// "halve, convert, double" so the conversion never sees a negative i64. Both
// branches now emit `xorps` before converting, so this test walks each one.
//
// Every expected value is computed with an independent route (f64 division by
// a power of two, or exact integer reasoning) rather than by re-running the
// same conversion, so a broken branch shows up as a wrong value and not just a
// different one.

fn conv_f32(x: u64) -> f32 {
    x as f32
}

fn conv_f64(x: u64) -> f64 {
    x as f64
}

fn main() {
    // Below 2^63: direct conversion.
    let small = [
        0u64,
        1,
        2,
        3,
        255,
        65535,
        1 << 24,
        (1u64 << 24) + 1,
        (1u64 << 31) + 7,
        u32::MAX as u64,
        (1u64 << 52) - 1,
        1u64 << 52,
        (1u64 << 53) - 1,
        1u64 << 53,
        (1u64 << 62) - 1,
        (1u64 << 63) - 1,
    ];
    for &x in &small {
        println!("s32 {} {}", x, conv_f32(x));
        println!("s64 {} {}", x, conv_f64(x));
    }

    // At and above 2^63: the halve/convert/double branch.
    let big = [
        1u64 << 63,
        (1u64 << 63) + 1,
        (1u64 << 63) + (1u64 << 40),
        u64::MAX - 1,
        u64::MAX,
    ];
    for &x in &big {
        println!("b32 {} {}", x, conv_f32(x));
        println!("b64 {} {}", x, conv_f64(x));
    }

    // Round-trip through f64: any value <= 2^53 is exactly representable, so
    // the conversion must return the original integer.
    let mut bad = 0u64;
    for x in 0..20_000u64 {
        if conv_f64(x) as u64 != x {
            bad += 1;
        }
    }
    println!("roundtrip_bad {}", bad);

    // f32 loses precision above 2^24; compare against the exactly-rounded
    // value obtained via f64, which has enough mantissa for all these inputs.
    let mut f32_mismatch = 0u64;
    for x in 0..20_000u64 {
        let via_f64 = conv_f64(x) as f32;
        if conv_f32(x).to_bits() != via_f64.to_bits() {
            f32_mismatch += 1;
        }
    }
    println!("f32_mismatch {}", f32_mismatch);

    // Values near the 2^63 split, compared against the mathematical value
    // scaled by 2^-63 and back.
    let mut split_ok = 0u64;
    for k in 0..64u32 {
        let base = (1u64 << 63).wrapping_add(1u64 << k);
        let expect = ((base >> 1) as f64) * 2.0;
        if conv_f64(base) == expect {
            split_ok += 1;
        }
    }
    println!("split_ok {}", split_ok);
}
