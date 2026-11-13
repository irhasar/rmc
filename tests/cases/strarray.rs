// A `&str` is a one-word pointer, so an array of them is an array of pointers
// and behaves like any other array: it can be indexed, iterated, mutated and
// passed to a function. This is the shape of a table of names, a table of
// messages, or the argument vector.
fn pick(a: [&str; 3], i: usize) -> &str {
    a[i]
}

fn main() {
    let names = ["alpha", "beta", "gamma"];
    let mut i = 0;
    while i < 3 {
        println!("{}", names[i]);
        i += 1;
    }

    let ann: [&str; 3] = ["one", "two", "three"];
    println!("{} {}", ann[0], ann[2]);

    let rep = ["x"; 4];
    println!("{} {} {}", rep[0], rep[1], rep[3]);

    let mut muts = ["a", "b", "c"];
    muts[1] = "B";
    println!("{} {} {}", muts[0], muts[1], muts[2]);

    println!("{}", names[0] == "alpha");
    println!("{}", pick(names, 2));
    println!("{}", names.len());

    for s in names {
        println!("[{}]", s);
    }

    let idx: usize = 1;
    println!("{}", names[idx]);

    // A general reference is also one word, so `[&i64; n]` is an array of
    // pointers and can be built from the addresses of locals.
    let a = 10;
    let b = 20;
    let refs = [&a, &b];
    let mut sum = 0;
    let mut j = 0;
    while j < 2 {
        sum += *refs[j];
        j += 1;
    }
    println!("{} {}", sum, *refs[1]);
}
