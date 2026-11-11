// Rust twin of whilelet.rms.
#[derive(Copy, Clone)]
enum Opt { None, Some(i64) }

fn step(k: i64) -> Opt {
    if k < 5 {
        Opt::Some(k * 10)
    } else {
        Opt::None
    }
}

fn first_div(c: i64) -> i64 {
    if c > 0 { 1 } else { return 2 }
}

fn main() {
    let mut i: i64 = 0;
    let mut sum: i64 = 0;
    while let Opt::Some(v) = step(i) {
        sum += v;
        i += 1;
    }
    println!("{}", sum);
    println!("{}", i);

    let mut j: i64 = 10;
    while let Opt::Some(v) = step(j) {
        println!("{}", v);
        j += 1;
    }
    println!("{}", j);

    let mut k: i64 = 0;
    let mut odd: i64 = 0;
    while let Opt::Some(v) = step(k) {
        k += 1;
        if v % 20 == 0 {
            continue;
        }
        odd += v;
    }
    println!("{}", odd);
    println!("{}", k);

    println!("{}", first_div(3));
    println!("{}", first_div(-3));
}
