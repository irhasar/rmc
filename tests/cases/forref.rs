// `for x in &arr` and `for x in &mut arr`: the loop variable is a reference to
// the element, so `*x` reads and `*x = v` writes the element in place.
struct P {
    x: i64,
    y: i64,
}

fn main() {
    let a = [10, 20, 30];
    for x in &a {
        print!("{} ", *x);
    }
    println!();

    // `&mut` reaches the original binding: the element is written in place.
    let mut b = [1, 2, 3];
    for x in &mut b {
        *x *= 2;
    }
    println!("{} {} {}", b[0], b[1], b[2]);

    // A struct element is a real reference: field access through it reads the
    // original field (no copy of the array is made).
    let ps = [P { x: 1, y: 2 }, P { x: 3, y: 4 }];
    let mut t = 0;
    for p in &ps {
        t += p.x + p.y;
    }
    println!("{}", t);

    // A `&mut` field write lands in the array.
    let mut qs = [P { x: 5, y: 6 }, P { x: 7, y: 8 }];
    for p in &mut qs {
        p.x += 10;
    }
    println!("{} {} {} {}", qs[0].x, qs[0].y, qs[1].x, qs[1].y);

    // Nested: the outer element is a reference to a row, the inner iterates it.
    let m = [[1, 2], [3, 4]];
    let mut s = 0;
    for row in &m {
        for x in row {
            s += x;
        }
    }
    println!("{}", s);

    // `continue` / `break` still work in a by-reference loop.
    let c = [1, 2, 3, 4, 5];
    for x in &c {
        if *x == 2 {
            continue;
        }
        if *x == 5 {
            break;
        }
        print!("{} ", *x);
    }
    println!();

    // An element place, `&a[i]`, still iterates as a sub-array.
    let g = [[1, 2, 3], [4, 5, 6]];
    let mut u = 0;
    for x in &g[1] {
        u += *x;
    }
    println!("{}", u);

    // The by-value form is unchanged.
    for x in a {
        print!("{} ", x);
    }
    println!();
    let mut z = 0;
    for _ in &a {
        z += 1;
    }
    println!("{}", z);

    // `arr.iter()` / `arr.iter_mut()` are the same by-reference loops.
    let it = [2, 4, 6];
    let mut isum = 0;
    for x in it.iter() {
        isum += *x;
    }
    println!("{}", isum);
    let mut im = [1, 1, 1];
    for x in im.iter_mut() {
        *x += isum;
    }
    println!("{} {} {}", im[0], im[1], im[2]);
}
