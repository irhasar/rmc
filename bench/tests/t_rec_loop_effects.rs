//! The recursion-to-loop pass must not delete a base case that has effects.
static mut BASE_WRITES: u64 = 0;
static mut LAST_BASE: u64 = u64::MAX;

#[inline(never)]
fn fib(n: u64) -> u64 {
    if n < 2 {
        unsafe {
            BASE_WRITES += 1;
            LAST_BASE = n;
        }
        n
    } else {
        fib(n - 1) + fib(n - 2)
    }
}

fn main() {
    let result = fib(10);
    unsafe {
        println!("{result} {} {}", BASE_WRITES > 0, LAST_BASE <= 1);
    }
}
