// A variant pattern argument may be a nested variant pattern (`O::W(I::A)`),
// a scalar test (`E::A(1)`, `E::A(1 | 2)`), or a wildcard, and the arm matches
// only when the outer tag and every refutable sub-pattern agree. A match that
// enumerates every variant of the outer enum and — for each payload arm with a
// sub-pattern — every variant of the payload enum is total without a `_`.

#[derive(Copy, Clone)]
enum I {
    A,
    B,
}

#[derive(Copy, Clone)]
enum O {
    W(I),
    Z,
}

#[derive(Copy, Clone)]
enum P {
    V(O),
    Q,
}

fn outer(o: O) -> i64 {
    match o {
        O::W(I::A) => 1,
        O::W(I::B) => 2,
        O::Z => 3,
    }
}

fn deep(p: P) -> i64 {
    match p {
        P::V(O::W(I::A)) => 10,
        P::V(O::W(I::B)) => 20,
        P::V(O::Z) => 30,
        P::Q => 40,
    }
}

fn nested_bind(o: O) -> i64 {
    match o {
        O::W(I::A) => 0,
        O::W(I::B) => 0,
        O::Z => 0,
    }
}

#[derive(Copy, Clone)]
enum E {
    A(i64),
    B(i64),
    C,
}

fn scalar(e: E) -> i64 {
    match e {
        E::A(1) => 10,
        E::A(_) => 11,
        E::B(2 | 3) => 20,
        E::B(_) => 21,
        E::C => 30,
    }
}

fn wildcard_covers(e: E) -> i64 {
    match e {
        E::A(_) => 1,
        E::B(_) => 2,
        E::C => 3,
    }
}

fn via_ref(o: &O) -> i64 {
    match o {
        O::W(I::A) => 100,
        O::W(I::B) => 200,
        O::Z => 300,
    }
}

fn main() {
    println!("{}", outer(O::W(I::A)));
    println!("{}", outer(O::W(I::B)));
    println!("{}", outer(O::Z));

    println!("{}", deep(P::V(O::W(I::A))));
    println!("{}", deep(P::V(O::W(I::B))));
    println!("{}", deep(P::V(O::Z)));
    println!("{}", deep(P::Q));

    println!("{}", nested_bind(O::Z));

    println!("{}", scalar(E::A(1)));
    println!("{}", scalar(E::A(9)));
    println!("{}", scalar(E::B(2)));
    println!("{}", scalar(E::B(3)));
    println!("{}", scalar(E::B(4)));
    println!("{}", scalar(E::C));

    println!("{}", wildcard_covers(E::A(7)));
    println!("{}", wildcard_covers(E::B(8)));
    println!("{}", wildcard_covers(E::C));

    println!("{}", via_ref(&O::W(I::A)));
    println!("{}", via_ref(&O::W(I::B)));
    println!("{}", via_ref(&O::Z));

    let o: O = O::W(I::B);
    if let O::W(I::B) = o {
        println!("iflet b");
    }
    if let O::W(I::A) = o {
        println!("iflet a");
    } else {
        println!("iflet not-a");
    }

    let mut m: O = O::W(I::A);
    while let O::W(I::A) = m {
        println!("whilelet a");
        m = O::Z;
    }
    println!("done");
}
