// The Rust twin of `strparam.rms`. rmc's `str` is a bare one-word pointer, so
// its functions take `str` by value; Rust's is a fat pointer, so the twin takes
// `&str` (and the struct holds a `&'static str`). `len()` returns `usize`, cast
// to `i64` to line up with rmc's `i64`.
fn pi(x: i64) {
    println!("{}", x);
}

fn classify(s: &str) -> i64 {
    if s == "add" { return 1; }
    if s == "sub" { return 2; }
    if s.len() == 0 { return 3; }
    return 0;
}

fn pick(s: &str) -> &str {
    return s;
}

#[derive(Copy, Clone)]
struct Msg { text: &'static str, id: i64 }

fn msg_text(m: Msg) -> &'static str {
    return m.text;
}

fn main() {
    pi(classify("add"));
    pi(classify("sub"));
    pi(classify(""));
    pi(classify("nope"));
    let s: &str = pick("hello");
    println!("{}", s);
    pi(s.len() as i64);
    let m = Msg { text: "world", id: 7 };
    println!("{}", msg_text(m));
    pi(m.text.len() as i64 + m.id);
}
