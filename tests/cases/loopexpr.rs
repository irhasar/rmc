fn find(n: i64) -> i64 {
    let mut i: i64 = 0;
    loop {
        if i * i >= n {
            break i;
        }
        i += 1;
    }
}

fn dbl(x: i64) -> i64 {
    x * 2
}

fn spin(flag: i64) -> i64 {
    if flag > 0 {
        loop {
            return -1;
        }
    }
    7
}

fn negate(x: i64) -> bool {
    let b: bool = !loop {
        break x > 0;
    };
    b
}

struct P {
    x: i64,
    y: i64,
}

enum E {
    V(P),
    W,
}

fn from_loop(n: i64) -> i64 {
    let e: E = E::V(loop {
        break P { x: n, y: n + 1 };
    });
    let r: i64 = match e {
        E::V(p) => p.x + p.y,
        E::W => 0,
    };
    r
}

fn array_from_loop() -> i64 {
    let mut i: i64 = 0;
    let a: [i64; 3] = loop {
        i += 1;
        if i == 2 {
            break [7, 8, 9];
        }
    };
    a[0] + a[1] + a[2]
}

fn main() {
    let a: i64 = loop {
        break 5;
    };
    println!("{}", a);

    let mut k: i64 = 0;
    let b: i64 = loop {
        k += 1;
        if k == 7 {
            break k * 10;
        }
    };
    println!("{}", b);
    println!("{}", k);

    let mut i: i64 = 0;
    let outer: i64 = loop {
        i += 1;
        let inner: i64 = loop {
            if i > 3 {
                break i * 100;
            }
            break 1;
        };
        if i == 5 {
            break inner + i;
        }
    };
    println!("{}", outer);

    let mut m: i64 = 0;
    let c: i64 = loop {
        m += 1;
        if m == 4 {
            break m;
        }
    } * 10
        + 1;
    println!("{}", c);
    println!(
        "{}",
        dbl(loop {
            break 21;
        })
    );

    println!("{}", find(20));
    println!("{}", find(0));
    println!("{}", find(100));

    println!("{}", spin(0));

    let mut n: i64 = 0;
    loop {
        n += 1;
        if n == 3 {
            break;
        }
    }
    let u: () = loop {
        n += 1;
        if n == 6 {
            break;
        }
    };
    println!("{}", n);

    if negate(3) {
        println!("1");
    } else {
        println!("0");
    }
    if negate(-1) {
        println!("1");
    } else {
        println!("0");
    }
    println!("{}", from_loop(10));
    println!("{}", from_loop(-4));
    println!("{}", array_from_loop());
}
