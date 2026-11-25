// The escape walk must reach the `stack_addr` however many arithmetic steps sit
// between it and the call: a fixed depth budget stopped at eight and left the
// slot looking private, so its initialization was deleted before the callee read
// it. The second case reaches one shared subexpression twice from a single
// argument, so a walk that skipped a value it had already seen would miss the
// slot as well.
#[inline(never)]
fn read_through(p: *const u64) -> u64 {
    let v = unsafe { *p };
    // Enough work that the callee is not worth inlining into `main`.
    v.wrapping_mul(3).wrapping_add(1)
}

// The slot is reachable only through a chain longer than the old budget.
#[inline(never)]
fn deep_chain(n: usize) -> u64 {
    let a: u64 = 42;
    let mut p: usize = &a as *const u64 as usize;
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    read_through(p as *const u64)
}

// `mid` is reached twice from one argument: at the end of the long chain and
// directly.
#[inline(never)]
fn shared_paths(n: usize) -> u64 {
    let a: u64 = 42;
    let s: usize = &a as *const u64 as usize;
    let mid = s ^ n;
    let mut p = mid;
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    p ^= n;
    p = p.wrapping_add(n);
    read_through((p ^ mid ^ mid) as *const u64)
}

fn main() {
    // 0 at run time, unknown to the compiler: nothing above can be folded away.
    let n = std::env::args().count() as usize - 1;
    // 42*3 + 1 = 127 for both.
    println!("{} {}", deep_chain(n), shared_paths(n));
}
