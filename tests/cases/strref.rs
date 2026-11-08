// Differential twin of `strref.rms`: `&str`/`&'static str` fields and
// parameters, `str` equality, and an array length written `N as usize`. rmc
// models `str` and `&str` identically, so the `.rms` twin spells them `str`.
const N: i64 = 4;

#[derive(Copy, Clone)]
struct Msg { text: &'static str, id: i64 }

fn text_of(m: Msg) -> &'static str { m.text }

fn first(s: &str) -> &str { s }

fn main() {
    let a: [i64; N as usize] = [10, 20, 30, 40];
    let m = Msg { text: "hello", id: 7 };
    println!("{}", text_of(m));
    println!("{}", first("world"));
    println!("{}", a[0] + a[3]);
    println!("{}", m.id);
    println!("{}", (text_of(m) == "hello") as i64);
    println!("{}", (text_of(m) != "hello") as i64);
}
