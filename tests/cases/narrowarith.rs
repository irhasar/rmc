// Narrow integer widths (`u8`/`u16`/`u32` and their signed twins) wrap at
// their own width, not at 64 bits. Every arithmetic here happens inside a
// function on a parameter, so rustc's constant-propagation lint does not
// reject it; the values still overflow a 64-bit register if the width is lost,
// which pins the truncation on the direct path, through a call, and through an
// inlined call.

fn add8(a: u8, b: u8) -> u8 {
    a + b
}

fn sub8(a: u8, b: u8) -> u8 {
    a - b
}

fn mul8(a: u8, b: u8) -> u8 {
    a * b
}

fn add8_lit(a: u8) -> u8 {
    a + 100
}

fn mul8_3(a: u8) -> u8 {
    a * 3
}

fn add16(a: u16, b: u16) -> u16 {
    a + b
}

fn mul16(a: u16, b: u16) -> u16 {
    a * b
}

fn add32(a: u32, b: u32) -> u32 {
    a + b
}

fn addi8(a: i8, b: i8) -> i8 {
    a + b
}

fn addi16(a: i16, b: i16) -> i16 {
    a + b
}

fn main() {
    // Each call passes literals whose 64-bit sum overflows, so a missing
    // truncation prints the wide value instead of the wrapped one.
    println!("{}", add8(200, 100));
    println!("{}", sub8(10, 20));
    println!("{}", mul8(200, 3));
    println!("{}", add8_lit(200));
    println!("{}", mul8_3(100));
    println!("{}", add16(60000, 10000));
    println!("{}", mul16(1000, 100));
    println!("{}", add32(4000000000, 400000000));
    println!("{}", addi8(100, 100));
    println!("{}", addi16(30000, 10000));

    // A cast truncates, and a later arithmetic op wraps again.
    let h: i64 = 300;
    let i: u8 = h as u8;
    println!("{}", i);
    println!("{}", (i as i64) + 100);

    // A narrow value cast up sees the wrapped value, not the 64-bit one.
    println!("{}", add8(200, 100) as i64);
}
