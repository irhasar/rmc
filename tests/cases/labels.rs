enum Opt { None, Some(i64) }

fn main() {
    // `break 'outer` from a `while` nested in a labelled `loop`.
    let mut i: i64 = 0;
    'outer: loop {
        let mut j: i64 = 0;
        while j < 10 {
            j = j + 1;
            i = i + 1;
            if i == 5 {
                break 'outer;
            }
        }
    }
    println!("{}", i);

    // `continue 'a` on an outer `for` from an inner `for`.
    let mut s: i64 = 0;
    'a: for _x in 0..3 {
        for y in 0..3 {
            if y == 1 {
                continue 'a;
            }
            s = s + 1;
        }
    }
    println!("{}", s);

    // A labelled `for` over an array, broken out of early.
    let xs: [i64; 4] = [1, 2, 3, 4];
    let mut acc: i64 = 0;
    'o: for x in xs {
        if x == 3 {
            break 'o;
        }
        acc = acc + x;
    }
    println!("{}", acc);

    // A labelled `loop` that yields a value through `break 'l`.
    let mut k: i64 = 0;
    let v: i64 = 'l: loop {
        k = k + 1;
        if k == 7 {
            break 'l k * 2;
        }
    };
    println!("{}", v);

    // A labelled `while` broken out of early.
    let mut w: i64 = 0;
    'w: while w < 10 {
        w = w + 1;
        if w == 4 {
            break 'w;
        }
    }
    println!("{}", w);

    // A label on `while let` attaches to the `loop` it desugars to, so
    // `break 'a` / `continue 'a` in the body target it.
    let mut e: Opt = Opt::Some(1);
    let mut n: i64 = 0;
    'a: while let Opt::Some(x) = e {
        n += x;
        if n >= 3 {
            break 'a;
        }
        e = Opt::Some(n);
    }
    println!("{}", n);

    // A `break 'b` from an inner `while` leaves the labelled `while let`.
    let mut m: i64 = 0;
    let mut f: Opt = Opt::Some(1);
    'b: while let Opt::Some(x) = f {
        while true {
            m += x;
            break 'b;
        }
    }
    println!("{}", m);
}
