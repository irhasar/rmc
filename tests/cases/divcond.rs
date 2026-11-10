// A nested loop whose condition cannot complete (`break 'o` before the `true`)
// makes its body unreachable, so a `continue 'o` there does not write the outer
// binding once per iteration: the outer `loop` still runs the write exactly
// once and the program is accepted. A condition that can complete leaves the
// body reachable, so the same `continue` would repeat the write (rejected).
fn break_in_cond() -> i64 {
    let x: i64;
    'o: loop {
        x = 1;
        while { break 'o; true } { continue 'o; }
    }
    x
}

fn empty_body() -> i64 {
    let x: i64;
    'o: loop {
        x = 2;
        while { break 'o; true } { }
    }
    x
}

fn plain_continue() -> i64 {
    let x: i64;
    'o: loop {
        x = 3;
        while { break 'o; true } { continue; }
    }
    x
}

fn main() {
    println!("{}", break_in_cond());
    println!("{}", empty_body());
    println!("{}", plain_continue());
}
