// Arrays of a payload enum. The element is an aggregate (`{ tag, payload }`),
// so an element read is addressed, not a single word, and the array stride is
// the element's size. rustc accepts these; rmc used to reject the element type
// outright.

#[derive(Copy, Clone)]
enum E {
    A(i64),
    B(i64, i64),
    C,
}

fn val(e: E) -> i64 {
    match e {
        E::A(x) => x,
        E::B(x, y) => x * 10 + y,
        E::C => -1,
    }
}

#[derive(Copy, Clone)]
struct P {
    x: i64,
    y: i64,
}

#[derive(Copy, Clone)]
enum W {
    S(P),
    N,
}

fn main() {
    // A repeat literal of a payload-enum element.
    let a = [E::C; 3];
    let mut i = 0;
    while i < 3 {
        println!("{}", val(a[i as usize]));
        i += 1;
    }

    // A list literal, then assignment into an element.
    let mut b = [E::A(1), E::B(2, 3), E::C];
    b[1 as usize] = E::B(4, 5);
    b[2 as usize] = E::A(9);
    println!("{}", b.len());
    let mut j = 0;
    while j < 3 {
        println!("{}", val(b[j as usize]));
        j += 1;
    }

    // A payload that is itself an aggregate (a struct) widens the stride.
    let mut w = [W::N, W::S(P { x: 1, y: 2 })];
    w[0 as usize] = W::S(P { x: 7, y: 8 });
    let mut k = 0;
    let mut sum = 0;
    while k < 2 {
        match w[k as usize] {
            W::S(p) => sum += p.x + p.y,
            W::N => sum += 100,
        }
        k += 1;
    }
    println!("{}", sum);
}
