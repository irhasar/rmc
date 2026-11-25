// The escape walk must reach the `stack_addr` however many arithmetic steps sit
// between it and the call: a fixed depth budget stopped at eight and left the
// slot looking private, so its initialization was deleted before the callee read
// it. Both chains below start from one shared subexpression, so the walk also
// has to record a value it meets through more than one path.
#[inline(never)]
fn read_through(p: *const u64) -> u64 {
    let v = unsafe { *p };
    // Enough work that the callee is not worth inlining into `main`.
    v.wrapping_mul(3).wrapping_add(1)
}

fn main() {
    let a: u64 = 42;
    // 0 at run time, unknown to the compiler: nothing below can be folded away.
    let n = std::env::args().count() as usize - 1;
    let s: usize = &a as *const u64 as usize;
    let mid = s ^ n;
    // Nine more alternating steps on top of `mid`: more than the old budget.
    let mut p1 = mid;
    let mut p2 = mid;
    p1 ^= n;
    p1 = p1.wrapping_add(n);
    p2 = p2.wrapping_add(n);
    p2 ^= n;
    p1 ^= n;
    p1 = p1.wrapping_add(n);
    p2 = p2.wrapping_add(n);
    p2 ^= n;
    p1 ^= n;
    p1 = p1.wrapping_add(n);
    p2 = p2.wrapping_add(n);
    p2 ^= n;
    p1 ^= n;
    p1 = p1.wrapping_add(n);
    p2 = p2.wrapping_add(n);
    p2 ^= n;
    p1 ^= n;
    p1 = p1.wrapping_add(n);
    p2 = p2.wrapping_add(n);
    p2 ^= n;
    // 42*3 + 1 = 127 for both.
    println!("{} {}", read_through(p1 as *const u64), read_through(p2 as *const u64));
}
