#![no_std]
#![no_main]
include!("prelude.rs");

// A realistic workload (nqueens), shared with `tests/real/`. Each of these
// programs exposed at least one back-end bug when it was written.
// Backtracking search: N-queens with bitmasks, the shape of a constraint
// solver (sudoku, scheduling, SAT-style search).
fn solve(n: i64, row: i64, cols: i64, d1: i64, d2: i64) -> i64 {
    if row == n {
        return 1;
    }
    let mut count: i64 = 0;
    let full = (1 << n) - 1;
    let mut avail = !(cols | d1 | d2) & full;
    while avail != 0 {
        let bit = avail & (0 - avail);
        avail ^= bit;
        count += solve(n, row + 1, cols | bit, ((d1 | bit) << 1) & full, (d2 | bit) >> 1);
    }
    count
}

fn run(n: i64) {
    let n = if n == 0 { 13 } else { n };
    out(solve(n, 0, 0, 0, 0));
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    run(arg(1));
    0
}
