#[derive(Copy, Clone)]
enum Opt { None, Some(i64) }

#[derive(Copy, Clone)]
struct P { x: i64 }

fn h(x: i64) -> i64 { x + 1 }
fn g() -> i64 { h(41) }
fn f(a: i64, b: i64) -> i64 { a * 100 + b }

fn make() -> P { println!("100"); P { x: 5 } }

fn nxt() -> i64 { println!("9"); 0 }

fn add_two(mut n: i64) -> i64 { n += 2; n }

impl Opt {
    fn tag_value(self) -> i64 {
        match self {
            Opt::Some(v) => v,
            Opt::None => 0,
            _ => 0,
        }
    }
}

fn main() {
    println!("{}", f(7, { let _e = Opt::Some(g()); 9 }));
    make().x += 1;
    let mut a: [i64; 2] = [1, 2];
    a[nxt() as usize] += 5;
    println!("{}", a[0]);
    println!("{}", Opt::Some(42).tag_value());
    println!("{}", add_two(3));
}
