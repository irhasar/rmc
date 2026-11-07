// rmc's print_bool writes 1 or 0, so the twin mirrors that with `pb`.
fn pb(b: bool) {
    println!("{}", b as i64);
}
fn main() {
    // Literal against literal.
    pb("abc" == "abc");
    pb("abc" == "abd");
    pb("abc" != "abc");
    pb("abc" != "abd");
    pb("abc" < "abd");
    pb("abd" < "abc");
    pb("abc" < "abc");
    pb("ab" < "abc");
    pb("abc" < "ab");
    pb("" < "a");
    pb("a" < "");
    pb("abc" <= "abc");
    pb("abc" >= "abc");
    pb("abc" <= "abd");
    pb("abd" >= "abc");
    pb("" == "");
    // Through variables, which is where a pointer compare would show up.
    let s = "hello";
    let t = "hello";
    let u = "world";
    pb(s == t);
    pb(s == u);
    pb(s != u);
    pb(s < u);
    pb(u < s);
    pb(s == "hello");
    pb("hello" == s);
    // A shared prefix longer than one byte.
    pb("abcdef" < "abcdeg");
    pb("abcdeg" < "abcdef");
    pb("abc" < "abcdef");
    pb("abcdef" <= "abcdef");
    pb("abcdef" >= "abcdef");
}
