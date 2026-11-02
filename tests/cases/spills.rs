// rmc-only runtime builtins, shimmed so this twin also builds under rustc.
fn alloc(_n: i64) -> i64 { 0 }
fn free(_p: i64) {}

fn mix(a: i64, b: i64) -> i64 {
    (a + b) + (a * b) + (a - b) + (b - a) + (a + a) + (b + b)
}

fn deep(a: i64, b: i64) -> i64 {
    ((a + b) + (a * b)) + ((a - b) + (b - a)) + ((a + a) + (b + b))
        + ((a * a) + (b * b)) + ((a - 1) + (b + 1)) + ((a + 2) - (b - 2))
}

fn heap(a: i64, b: i64) -> i64 {
    let p = alloc(16);
    let q = alloc(24);
    free(p);
    free(q);
    (a + b) + (a * b) + p - p + (a - b) + (b - a) + q - q
}

fn main() {
    println!("{}", mix(6, 7));
    println!("{}", mix(-5, 9));
    println!("{}", deep(3, 4));
    println!("{}", deep(-8, 5));
    println!("{}", heap(6, 7));
    println!("{}", heap(-5, 9));
}
