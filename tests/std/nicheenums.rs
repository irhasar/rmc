// A niche-optimised enum (`Option<&T>`) carries no tag word; rustc folds
// `discriminant(_x) == 1` for `is_some`. rmc must use rustc's declaration-order
// discriminants (`None`=0, `Some`=1; `Ok`=0, `Err`=1), not the order variants
// happen to be constructed in, or `is_some` reads false.
fn main() {
    let a = [10, 20, 30];
    println!("{} {}", a.get(0).is_some(), a.get(9).is_none());
    println!("{}", a.get(1).is_some());
    let r: Result<i64, &str> = Ok(5);
    println!("{} {}", r.is_ok(), r.is_err());
    let e: Result<i64, &str> = Err("bad");
    println!("{} {}", e.is_ok(), e.is_err());
    match a.get(2) {
        Some(v) => println!("got {}", v),
        None => println!("none"),
    }
    match a.get(5) {
        Some(v) => println!("got {}", v),
        None => println!("none"),
    }
    println!("{}", a.get(0).unwrap_or(&-1));
}
