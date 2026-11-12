// Narrow-width overflow traps. Every operand is a function parameter, so rustc
// does not constant-fold and reject the call at compile time; the overflow is a
// runtime panic under `-C overflow-checks=on` (the `.overflow` marker turns the
// same check on for rmc). A narrow `u8`/`u16`/`u32` result can exceed its range
// without overflowing 64 bits (`250u8 + 10` wraps to `4` in a 64-bit register),
// so the trap must test the declared width, not just the `jo` flag. The argv
// selects one operation per run, so each trap site is exercised on its own.

fn add8(a: u8, b: u8) -> u8 {
    a + b
}

fn sub8(a: u8, b: u8) -> u8 {
    a - b
}

fn mul8(a: u8, b: u8) -> u8 {
    a * b
}

fn add16(a: u16, b: u16) -> u16 {
    a + b
}

fn add32(a: u32, b: u32) -> u32 {
    a + b
}

fn addi8(a: i8, b: i8) -> i8 {
    a + b
}

fn shl8(x: u8, n: u8) -> u8 {
    x << n
}

fn shr8(x: u8, n: u8) -> u8 {
    x >> n
}

fn shl16(x: u16, n: i64) -> u16 {
    x << n
}

fn neg8(a: i8) -> i8 {
    -a
}

fn main() {
    let which: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    if which == 0 {
        println!("{}", add8(250, 10));
    } else if which == 1 {
        println!("{}", sub8(5, 10));
    } else if which == 2 {
        println!("{}", mul8(20, 20));
    } else if which == 3 {
        println!("{}", add16(65535, 1));
    } else if which == 4 {
        println!("{}", add32(4294967295, 1));
    } else if which == 5 {
        println!("{}", addi8(127, 1));
    } else if which == 6 {
        println!("{}", shl8(1, 8));
    } else if which == 7 {
        println!("{}", shr8(128, 8));
    } else if which == 8 {
        println!("{}", shl16(1, 16));
    } else if which == 9 {
        println!("{}", neg8(-128));
    } else if which == 10 {
        // In range: `u8` addition exactly at the boundary, a `u8` shift by 7 and
        // a shift by 0 must not trap.
        println!("{}", add8(255, 0));
        println!("{}", shl8(1, 7));
        println!("{}", shl8(200, 0));
    } else {
        // A `u32` shift by 31 (the top bit) and a wide `i64` add must not trap.
        println!("{}", shl16(1, 15));
        println!("{}", add32(2147483648, 0));
    }
}
