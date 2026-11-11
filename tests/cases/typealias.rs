// `type` aliases: a transparent name for a type. A use is rewritten to the
// target at parse time, so an alias never reaches layout or codegen and a
// struct alias is the struct itself. An alias may be used before it is
// declared, and one alias may name another.
type N = i64;

fn main() {
    // A scalar alias in a `let`, a parameter and a return type.
    let a: N = 5;
    println!("{}", a + 1);
    println!("{}", add(2, 3));
    // A forward reference: `Later` is declared after this use.
    let f: Later = 9;
    println!("{}", f);
    // A chain: `A` names `B`, which names `i64`.
    let c: A = 7;
    println!("{}", c);
    // A struct alias is the struct itself, with the same layout and fields.
    let p: Pt = P { x: 4, y: 5 };
    println!("{}", p.x + p.y);
    // An alias for a tuple and for an array, the latter with a `const` length.
    let t: Pair = (3, 6);
    println!("{}", t.0 * t.1);
    // A forward alias *inside* a tuple: `PairF` names `Later`, declared below,
    // so the tuple's element layout must be rebuilt once the alias resolves.
    let pf: PairF = (8, 9);
    println!("{}", pf.0 + pf.1);
    let arr: Arr = [1, 2, 3, 4];
    println!("{}", arr[0] + arr[3]);
    // An alias behind a reference and in a cast.
    let x: N = 11;
    let r: &N = &x;
    println!("{}", *r);
    println!("{}", (300 as N) - 300);
    // An enum alias is the enum itself, matched by its variants.
    let e: E = Ev::B;
    match e {
        Ev::A => println!("{}", 0),
        Ev::B => println!("{}", 2),
    }
    // A nested-array alias, indexed twice.
    let grid: Grid = [[1, 2], [3, 4]];
    println!("{}", grid[1][0]);
}

fn add(a: N, b: N) -> N {
    return a + b;
}

type Later = i64;
type A = B;
type B = i64;

struct P {
    x: i64,
    y: i64,
}

enum Ev {
    A,
    B,
}

type Pt = P;
type Pair = (i64, i64);
type PairF = (Later, i64);
type Arr = [i64; 4];
type E = Ev;
type Grid = [[i64; 2]; 2];
