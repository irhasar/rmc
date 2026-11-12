// Integer-literal width suffixes (`200u8`, `1i64`, `0xffu32`) name the
// literal's width, exactly as rustc: `200u8` is `200 as u8` (i.e. `44`), the
// value takes part in arithmetic at that width (`200u8 + 100` overflows), and a
// magnitude that does not fit the width is a compile error. A native-width
// suffix (`5i64`) is a plain `i64`.

fn main() {
    let z = 200u8;
    let w: u8 = z;
    println!("{}", w);
    println!("{}", 255u8 as i64);
    println!("{}", 0xffu8);
    println!("{}", 100u8 + 100);
    println!("{}", 200u8 - 100);
    println!("{}", 3u8 * 30);
    println!("{}", 60000u16 - 1000);
    println!("{}", -128i8);
    println!("{}", 126i8 + 1);
    println!("{}", 5i64 + 2);
    println!("{}", 10u32 + 5);

    let x = 5u8;
    match x {
        5u8 => println!("{}", 1),
        200u8 => println!("{}", 2),
        _ => println!("{}", 3),
    }
}
