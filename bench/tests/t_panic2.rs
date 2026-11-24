fn may_fail(x: i64) -> Result<i64, String> { if x % 7 == 0 { Err(format!("bad {}", x)) } else { Ok(x*x) } }
fn main() {
    let mut ok = 0i64; let mut errs = 0usize; let mut first = String::new();
    for i in 0..5000i64 { match may_fail(i) { Ok(v)=>ok=ok.wrapping_add(v), Err(e)=>{ if errs==0 { first=e; } errs+=1; } } }
    println!("{} {} {}", ok, errs, first);
}
