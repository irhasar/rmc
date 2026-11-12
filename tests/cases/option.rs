// The injected `Option`/`Result` prelude: constructors, methods, and the
// `if let` / `while let` / `match` forms. Every line must match rustc's output.
fn half(x: i64) -> Option<i64> {
    if x % 2 == 0 { Some(x / 2) } else { None }
}

fn checked_div(a: i64, b: i64) -> Result<i64, i64> {
    if b == 0 { Err(-1) } else { Ok(a / b) }
}

fn main() {
    let a: Option<i64> = Some(10);
    let b: Option<i64> = None;
    println!("{}", a.is_some());
    println!("{}", a.is_none());
    println!("{}", b.is_some());
    println!("{}", b.is_none());

    println!("{}", a.unwrap());
    println!("{}", b.unwrap_or(-7));
    println!("{}", a.expect("has value"));

    if let Some(v) = a { println!("{}", v); } else { println!("{}", 0); }
    if let Some(v) = b { println!("{}", v); } else { println!("{}", 0); }

    let s = match a { Some(v) => v * 2, None => -1 };
    println!("{}", s);
    let t = match b { Some(v) => v * 2, None => -1 };
    println!("{}", t);

    let mut cur: Option<i64> = Some(3);
    while let Some(v) = cur {
        println!("{}", v);
        cur = if v > 0 { Some(v - 1) } else { None };
    }

    println!("{}", half(8).unwrap());
    println!("{}", half(7).unwrap_or(0));

    let r: Result<i64, i64> = checked_div(10, 2);
    println!("{}", r.is_ok());
    println!("{}", r.is_err());
    println!("{}", r.unwrap());

    let e: Result<i64, i64> = checked_div(10, 0);
    println!("{}", e.is_ok());
    println!("{}", e.is_err());
    println!("{}", e.unwrap_or(-99));
    println!("{}", e.err().unwrap());

    let bridged = b.ok_or(-5);
    println!("{}", bridged.unwrap_or(0));
    println!("{}", r.ok().unwrap());
}
