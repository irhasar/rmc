#[derive(Copy, Clone)]
struct P { x: i64, y: i64 }

fn sum_row(row: [i64; 3]) -> i64 {
    row[0] + row[1] + row[2]
}

fn sum_pairs(a: [P; 3]) -> i64 {
    a[0].x + a[1].y + a[2].x
}

fn main() {
    let m: [[i64; 3]; 2] = [[1, 2, 3], [4, 5, 6]];
    println!("{}", m[0][0]);
    println!("{}", m[0][2]);
    println!("{}", m[1][0]);
    println!("{}", m[1][2]);

    let mut i: i64 = 0;
    while i < 2 {
        let mut j: i64 = 0;
        while j < 3 {
            println!("{}", m[i as usize][j as usize]);
            j += 1;
        }
        i += 1;
    }

    let z: [[i64; 3]; 2] = [[7; 3]; 2];
    println!("{}", z[0][0]);
    println!("{}", z[1][2]);

    let big: [[i64; 3]; 4] = [[13; 3]; 4];
    let mut bi: i64 = 0;
    while bi < 4 {
        println!("{}", big[bi as usize][0] + big[bi as usize][1] + big[bi as usize][2]);
        bi += 1;
    }

    let sp: [P; 5] = [P { x: 4, y: 6 }; 5];
    let mut pi: i64 = 0;
    while pi < 5 {
        println!("{}", sp[pi as usize].x + sp[pi as usize].y);
        pi += 1;
    }

    let mut w: [[i64; 2]; 2] = [[0; 2]; 2];
    w[1][0] = 11;
    w[0][1] = 22;
    println!("{}", w[0][0]);
    println!("{}", w[0][1]);
    println!("{}", w[1][0]);
    println!("{}", w[1][1]);

    let src: [i64; 2] = [8, 9];
    w[0] = src;
    println!("{}", w[0][0]);
    println!("{}", w[0][1]);

    let mut a: [P; 3] = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }];
    println!("{}", a[0].x);
    println!("{}", a[1].y);
    println!("{}", a[2].x);
    a[1].x = 30;
    println!("{}", a[1].x);
    let q: P = P { x: 100, y: 200 };
    a[2] = q;
    println!("{}", a[2].x);
    println!("{}", a[2].y);

    let s: [P; 2] = [P { x: 9, y: 8 }; 2];
    println!("{}", s[0].x);
    println!("{}", s[1].y);

    println!("{}", sum_row(m[1]));
    println!("{}", sum_pairs(a));

    let mut acc: i64 = 0;
    for row in m {
        acc += row[0] + row[1] + row[2];
    }
    println!("{}", acc);
}
