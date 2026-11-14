// A `//` inside a string literal is data, not a comment. The MIR parser cut
// every line at the first `//`, so a URL or template containing one lost its
// tail: `"https://example.com"` became `"https:"`.
fn main() {
    println!("{}", "https://example.com/x");
    println!("{}", "see https://a.b/c for info");
    println!("{}", "a//b");
    // A `//` after a real comment marker is still a comment.
    println!("{}", "ok"); // trailing comment with http://x
    println!("{}", "https://x // not a comment");
    // Backslash escapes before a quote must not confuse the scanner.
    println!("{}", "quote \" then // here");
    println!("{}", "back\\slash // tail");
}
