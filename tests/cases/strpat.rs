// rmc's print_i64/print_str write the same text as `println!`.
fn pi(x: i64) {
    println!("{}", x);
}
fn ps(s: &str) {
    println!("{}", s);
}
fn main() {
    // A value-position match, with a catch-all.
    let s = "a";
    pi(match s { "a" => 1, "b" => 2, _ => 3 });
    pi(match s { "b" => 1, _ => 2 });
    // Several distinct literals, hit in the middle.
    let t = "b";
    pi(match t { "a" => 1, "b" => 2, _ => 3 });
    pi(match t { "a" => 1, "b" => 2, "c" => 3, _ => 4 });
    // An or-pattern over literals, sharing one body.
    pi(match t { "a" | "b" => 1, _ => 2 });
    pi(match t { "x" | "y" => 1, _ => 2 });
    // An empty literal is a valid pattern.
    let e = "";
    pi(match e { "" => 1, _ => 2 });
    pi(match e { "a" => 1, _ => 2 });
    // A prefix relationship, where a pointer compare and a content compare
    // could not both be right.
    let p = "abc";
    pi(match p { "ab" => 1, "abc" => 2, _ => 3 });
    // Statement position, unit arms.
    match s { "a" => ps("A"), _ => ps("other") };
    match s { "b" => ps("B"), _ => ps("not b") };
    // `if let`, both matched and not, with and without an `else`.
    if let "a" = s { pi(10); } else { pi(20); }
    if let "b" = s { pi(30); } else { pi(40); }
    if let "a" = s { pi(50); }
    // A match whose scrutinee is a literal directly.
    pi(match "hi" { "hi" => 1, _ => 2 });
    pi(match "ho" { "hi" => 1, _ => 2 });
}
