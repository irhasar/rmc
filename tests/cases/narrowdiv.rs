// Narrow division overflow. `i8::MIN / -1` (and `% -1`) has no representable
// `i8` quotient, so Rust traps — even though `-128 / -1` is fine at 64 bits and
// the 64-bit `idiv` would produce the right bits. The trap must therefore test
// the dividend's own width, not `i64::MIN`. It fires with or without the
// overflow flag, exactly as rustc, so this case needs no `.overflow` marker.
// The argv selects one operation per run; the non-trapping runs pin that the
// ordinary path is unaffected.

fn div8(a: i8, b: i8) -> i8 {
    a / b
}

fn rem8(a: i8, b: i8) -> i8 {
    a % b
}

fn div16(a: i16, b: i16) -> i16 {
    a / b
}

fn div32(a: i32, b: i32) -> i32 {
    a / b
}

fn div64(a: i64, b: i64) -> i64 {
    a / b
}

fn div8c(a: i8) -> i8 {
    a / -1
}

fn main() {
    let which: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    if which == 0 {
        println!("{}", div8(-128, -1));
    } else if which == 1 {
        println!("{}", rem8(-128, -1));
    } else if which == 2 {
        println!("{}", div16(-32768, -1));
    } else if which == 3 {
        println!("{}", div32(-2147483648, -1));
    } else if which == 4 {
        println!("{}", div64(-9223372036854775807 - 1, -1));
    } else if which == 5 {
        println!("{}", div8c(-128));
    } else if which == 6 {
        println!("{}", div8(-128, 2));
    } else if which == 7 {
        println!("{}", div8c(-100));
    } else if which == 8 {
        println!("{}", div32(-2147483648, 2));
    } else if which == 9 {
        println!("{}", div64(-9223372036854775807 - 1, 2));
    } else {
        // A divide by zero traps too (a separate path), and an ordinary divide
        // of a narrow negative is the wrapping-free case.
        println!("{}", div8(-127, -1));
    }
}
