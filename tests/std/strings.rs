// `&str` values and the operations `core` lowers to: Display of a local or a
// literal, `len()`, `==` (which becomes `compare_bytes`), and a `match` on a
// string literal (a promoted `&&str` constant). rmc models `&str` as a thin
// reference to a `{len: i64, bytes..}` record, so the runtime helpers must
// deref once and read the record's length word.
fn main() {
    let s = "hello";
    println!("{}", s);
    println!("{}", s.len());
    println!("{} {}", s, "world");

    let a = "x";
    let b = "x";
    println!("{}", a == b);

    match s {
        "hello" => println!("match"),
        _ => println!("no match"),
    }

    if "a" == "a" {
        println!("literal eq");
    }
}
