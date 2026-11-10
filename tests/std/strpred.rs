// `str::starts_with`/`ends_with`/`contains` lower to byte-slice predicates on
// the `&[u8]` cast of a `&str`. rmc implements those as runtime helpers that
// read its `{len, bytes..}` record. The substring test also passes a string
// *literal* straight as a call argument, which the string collector must still
// give a `.rodata` record (else the label falls back to the literal's text).
fn main() {
    let s = "hello world";
    println!("{} {} {}", s.starts_with("hello"), s.starts_with("world"), s.starts_with(""));
    println!("{} {} {}", s.ends_with("world"), s.ends_with("hello"), s.ends_with(""));
    println!("{} {} {}", s.contains("lo wo"), s.contains("xyz"), s.contains(""));
    println!("{}", "hi".starts_with("hello"));
    println!("{}", "hello".ends_with("hello"));
    let w = "abc";
    if w.contains("b") {
        println!("found");
    }
}
