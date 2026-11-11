// A trailing `;` may be omitted after an assignment or a compound assignment
// when it is the last thing in a block, and a stray `;` is an empty statement.
// Both match rustc.
fn set_to(x: &mut i64, v: i64) {
    // A `()`-returning body whose last statement is an assignment with no `;`.
    *x = v
}

fn bump(x: &mut i64) {
    *x += 1
}

fn main() {
    let mut x: i64 = 0;
    x = 1
    ;
    println!("{}", x);

    let mut y: i64 = 0;
    if y == 0 {
        y = 10
    } else {
        y = 20
    }
    println!("{}", y);

    // An assignment with no `;` as the last thing in a nested block.
    let mut t: i64 = 3;
    {
        t *= 4
    }
    println!("{}", t);

    // Empty statements between and after real ones.
    let mut s: i64 = 0;
    ;
    s += 1;
    s += 2
    ;
    println!("{}", s);

    let mut a: [i64; 3] = [0; 3];
    let mut i: i64 = 0;
    while i < 3 {
        a[i as usize] = i * i;
        i += 1
    }
    println!("{} {} {}", a[0], a[1], a[2]);

    let mut n: i64 = 0;
    while n < 3 {
        n += 1
    }
    println!("{}", n);

    let mut m: i64 = 0;
    loop {
        m += 1;
        if m == 4 {
            break
        }
    }
    println!("{}", m);

    let mut c: i64 = 0;
    match 2 {
        2 => c = 100,
        _ => c = 200,
    }
    println!("{}", c);

    let mut r: i64 = 5;
    set_to(&mut r, 7);
    bump(&mut r);
    println!("{}", r);
}
