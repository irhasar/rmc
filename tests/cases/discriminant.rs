// A plain Rust subset.
#[derive(Copy, Clone)]
enum Code { Ok = 0, Warn = 4, Err }
#[derive(Copy, Clone)]
enum Flags { A = 1, B = 2, C = 4, D = 8 }
#[derive(Copy, Clone)]
enum Signed { Neg = -3, Zero, Pos }
#[derive(Copy, Clone)]
enum Plain { X, Y, Z }
#[derive(Copy, Clone)]
enum Edge { Min = -9223372036854775808, Max = 9223372036854775807 }

fn code_val(c: Code) -> i64 {
    match c { Code::Ok => 100, Code::Warn => 200, Code::Err => 300 }
}

fn flag_sum(f: Flags) -> i64 {
    match f { Flags::A => 1, Flags::B => 2, Flags::C => 4, Flags::D => 8 }
}

fn main() {
    println!("{}", Code::Ok as i64);
    println!("{}", Code::Warn as i64);
    println!("{}", Code::Err as i64);
    println!("{}", Flags::D as i64);
    println!("{}", Signed::Neg as i64);
    println!("{}", Signed::Zero as i64);
    println!("{}", Signed::Pos as i64);
    println!("{}", Plain::Z as i64);

    println!("{}", code_val(Code::Ok) + code_val(Code::Warn) + code_val(Code::Err));
    println!("{}", flag_sum(Flags::A) + flag_sum(Flags::B) + flag_sum(Flags::C) + flag_sum(Flags::D));

    let c: Code = Code::Warn;
    let d: Code = c;
    println!("{}", d as i64);
    let arr: [Flags; 3] = [Flags::A, Flags::B, Flags::C];
    println!("{}", arr[0] as i64 + arr[1] as i64 + arr[2] as i64);

    println!("{}", (Code::Err as i64) - (Code::Ok as i64));
    println!("{}", (Flags::D as i64) - (Flags::A as i64));

    println!("{}", Edge::Min as i64);
    println!("{}", Edge::Max as i64);
}
