fn sum2(p: [i64; 2]) -> i64 {
    let [x, y] = p;
    x + y
}

fn main() {
    let a: [i64; 4] = [1, 2, 3, 4];

    let [w, x, y, z] = a;
    println!("{}", w * 1000 + x * 100 + y * 10 + z);

    let [_, b, _, d] = a;
    println!("{}", b * 10 + d);

    let [.., last] = a;
    println!("{}", last);
    let [first, ..] = a;
    println!("{}", first);
    let [f2, .., l2] = a;
    println!("{}", f2 * 10 + l2);

    let m: [[i64; 2]; 2] = [[5, 6], [7, 8]];
    let [[p, q], [r, s]] = m;
    println!("{}", p * 1000 + q * 100 + r * 10 + s);

    let [head, tail] = m;
    println!("{}", head[0] * 10 + head[1] + tail[0] + tail[1]);

    let mut acc: i64 = 0;
    for [u, v] in m {
        acc = acc + u * 10 + v;
    }
    println!("{}", acc);

    let mut acc2: i64 = 0;
    for [u, ..] in m {
        acc2 = acc2 + u;
    }
    println!("{}", acc2);

    let k: [i64; 2] = [9, 8];
    match k {
        [aa, bb] => println!("{}", aa * 10 + bb),
    }

    if let [aa, bb] = k {
        println!("{}", aa + bb);
    }

    let [mut mm, nn] = k;
    mm = mm + 100;
    println!("{}", mm + nn);

    println!("{}", sum2(k));
}
