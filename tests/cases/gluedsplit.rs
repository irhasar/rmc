// A nested `fn` and a block `const` declared *after* an item whose type closes
// two generic lists with a glued `>>`. Splitting that `>>` inserts a token, so
// the prescan's recorded item indices must shift with it; otherwise the `fn`
// and `const` are no longer recognised as items and are parsed as statements.
fn main() {
    let v: Vec<Vec<i64>> = Vec::new();
    fn helper() -> i64 { 7 }
    const K: i64 = 2;
    println!("{} {} {}", v.len(), helper(), K);

    let w: Vec<Vec<i64>> = Vec::new();
    fn twice(n: i64) -> i64 { n * 2 }
    println!("{} {}", w.len(), twice(21));
}
