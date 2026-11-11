#[derive(Copy, Clone)]
struct I { u: i64 }
#[derive(Copy, Clone)]
struct S { a: [i64; 2], b: I }

#[derive(Copy, Clone)]
enum E {
    Pair(i64, i64),
    Trip(i64, i64, i64),
    Hold(S),
    Arr([i64; 2]),
    Mix(i64, S, [i64; 2]),
    None,
}

fn wildcard(e: E) -> i64 {
    match e {
        E::Pair(_, y) => y,
        _ => 0,
    }
}

fn mutate(e: E) -> i64 {
    match e {
        E::Pair(mut x, y) => { x = x + y; x }
        _ => 0,
    }
}

fn rest_prefix(e: E) -> i64 {
    match e {
        E::Trip(x, ..) => x,
        _ => 0,
    }
}

fn rest_only(e: E) -> i64 {
    match e {
        E::Trip(..) => 1,
        E::None => 0,
        _ => 0 - 1,
    }
}

fn nested_struct(e: E) -> i64 {
    match e {
        E::Hold(S { a: [p, q], b: I { u } }) => p * 100 + q * 10 + u,
        _ => 0,
    }
}

fn nested_array(e: E) -> i64 {
    match e {
        E::Arr([a, b]) => a * 10 + b,
        _ => 0,
    }
}

fn mixed(e: E) -> i64 {
    match e {
        E::Mix(n, S { a: [p, q], b: I { u } }, [r, s]) => n + p + q + u + r + s,
        _ => 0,
    }
}

impl E {
    fn first(self) -> i64 {
        match self {
            E::Pair(x, ..) => x,
            E::Trip(x, ..) => x,
            _ => 0,
        }
    }
}

// A pattern reads through a reference to the scrutinee, as match ergonomics
// does: `&e` is dereferenced to the `E` underneath, so the test and every
// payload read go through the one deref.
fn via_ref(e: &E) -> i64 {
    match e {
        E::Pair(x, _) => x * 2,
        E::Trip(x, ..) => x * 3,
        _ => 0,
    }
}

fn unit_via_ref(e: &E) -> i64 {
    match e {
        E::None => 100,
        E::Pair(_, _) => 200,
        E::Trip(_, _, _) => 300,
        E::Hold(_) => 400,
        E::Arr(_) => 500,
        E::Mix(_, _, _) => 600,
    }
}

fn nested_via_ref(e: &E) -> i64 {
    match e {
        E::Hold(S { a: [p, q], b: I { u } }) => p * 100 + q * 10 + u,
        _ => 0,
    }
}

fn main() {
    println!("{}", wildcard(E::Pair(3, 4)));
    println!("{}", mutate(E::Pair(3, 4)));
    println!("{}", rest_prefix(E::Trip(5, 6, 7)));
    println!("{}", rest_only(E::Trip(1, 2, 3)));
    println!("{}", rest_only(E::None));
    println!("{}", nested_struct(E::Hold(S { a: [1, 2], b: I { u: 3 } })));
    println!("{}", nested_array(E::Arr([7, 8])));
    println!("{}", mixed(E::Mix(1, S { a: [2, 3], b: I { u: 4 } }, [5, 6])));
    println!("{}", E::Pair(9, 10).first());
    println!("{}", E::Trip(11, 12, 13).first());

    let e: E = E::Hold(S { a: [4, 5], b: I { u: 6 } });
    if let E::Hold(S { a: [p, q], b: I { u } }) = e {
        println!("{}", p + q + u);
    }

    println!("{}", via_ref(&E::Pair(7, 8)));
    println!("{}", via_ref(&E::Trip(9, 10, 11)));
    println!("{}", via_ref(&E::None));
    println!("{}", unit_via_ref(&E::None));
    println!("{}", unit_via_ref(&E::Arr([1, 2])));
    println!("{}", nested_via_ref(&E::Hold(S { a: [1, 2], b: I { u: 3 } })));

    let f: E = E::Pair(21, 22);
    if let E::Pair(x, y) = &f {
        println!("{}", x * 100 + y);
    }
}
