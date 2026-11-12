// `str::starts_with`, `str::ends_with` and `str::contains` compare bytes, not
// pointers, so each must look through the `{len, bytes}` record.
struct P {
    name: &'static str,
}

fn has_marker(s: &str) -> bool {
    s.contains("mark")
}

fn main() {
    let s = "hello world";

    // A prefix / suffix / substring at the start, the middle, the end and
    // absent.
    println!("{}", s.starts_with("hello"));
    println!("{}", s.starts_with("world"));
    println!("{}", s.ends_with("world"));
    println!("{}", s.ends_with("hello"));
    println!("{}", s.contains("lo w"));
    println!("{}", s.contains("xyz"));
    println!("{}", s.contains("hello world"));

    // The empty needle is a prefix, a suffix and a substring.
    println!("{}", s.starts_with(""));
    println!("{}", s.ends_with(""));
    println!("{}", s.contains(""));

    // A needle longer than the haystack.
    println!("{}", s.starts_with("hello world!"));
    println!("{}", s.ends_with("hello world!"));
    println!("{}", s.contains("hello world!"));

    // Single-byte needles, at both ends and in the middle.
    println!("{}", s.starts_with("h"));
    println!("{}", s.ends_with("d"));
    println!("{}", s.contains(" "));
    println!("{}", s.contains("z"));

    // A needle bound to a variable and a needle passed through a field.
    let needle = "wor";
    println!("{}", s.contains(needle));
    println!("{}", s.starts_with(needle));
    let p = P { name: "world" };
    println!("{}", p.name.ends_with("rld"));
    println!("{}", p.name.starts_with("wor"));

    // A `&str` receiver and a `&str` parameter.
    let r = &s;
    println!("{}", r.starts_with("he"));
    println!("{}", r.ends_with("ld"));
    println!("{}", has_marker(s));
    println!("{}", has_marker("a marker here"));

    // Receiver and argument are evaluated left to right, once each.
    println!("{}", pick("left").contains(pick("right")));
    println!("{}", pick("abc").starts_with(pick("ab")));
    println!("{}", pick("abc").ends_with(pick("xyz")));

    // Overlapping occurrences: `contains` only needs one.
    println!("{}", "aaaa".contains("aaa"));
    println!("{}", "abcabc".contains("bcab"));
    println!("{}", "abcabc".ends_with("abc"));
}

fn pick(s: &'static str) -> &'static str {
    print!("{} ", s);
    s
}
