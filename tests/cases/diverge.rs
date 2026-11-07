enum E { A, B }

fn all_return(e: E) -> i64 {
    match e {
        E::A => { return 1; }
        E::B => { return 2; }
    }
}

fn value_or_return(e: E) -> i64 {
    match e {
        E::A => 10,
        E::B => { return 20; }
    }
}

fn let_from_diverge(c: bool) -> i64 {
    let x: i64 = if c { return 1; } else { return 2; };
    return x;
}

fn return_diverge(e: E) -> i64 {
    return match e {
        E::A => { return 3; }
        E::B => { return 4; }
    };
}

fn main() {
    println!("{}", all_return(E::A));
    println!("{}", all_return(E::B));
    println!("{}", value_or_return(E::A));
    println!("{}", value_or_return(E::B));
    println!("{}", let_from_diverge(true));
    println!("{}", let_from_diverge(false));
    println!("{}", return_diverge(E::A));
    println!("{}", return_diverge(E::B));
}
