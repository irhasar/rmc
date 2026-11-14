fn main() {
    let a: i64 = 255;
    let b: u8 = 255;
    let c: u16 = 65535;
    let d: i32 = -1;
    let e: i8 = -1;
    let f: u64 = 4095;
    println!("{:x}", a);
    println!("{:x} {:o} {:b}", a, a, a);
    println!("{:x} {:o} {:b}", b, b, b);
    println!("{:x} {:o} {:b}", c, c, c);
    println!("{:x} {:o} {:b}", d, d, d);
    println!("{:x} {:o} {:b}", e, e, e);
    println!("hex={:x} oct={:o} bin={:b}", f, f, f);
    println!("upper={:X} lower={:x}", f, f);
}
