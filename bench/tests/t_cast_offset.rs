// A casted offset in front of another local's address must not hide that local
// from the escape analysis: the sum carries both slots, and a call that receives
// it can read either.
#[inline(never)]
fn read_through(p: *const u64) -> u64 {
    let v = unsafe { *p };
    // Enough work that the callee is not worth inlining into `main`.
    v.wrapping_mul(3).wrapping_add(1)
}

fn main() {
    // 8-aligned, so the low three address bits are zero and `low` is 0.
    let a: u64 = 7;
    let b: u64 = 42;
    let low = (((&raw const a) as usize as u8) & 7) as usize;
    // The sum's left operand derives from `a`, its right operand is `b`'s
    // address. Provenance has to report both slots, not just the first.
    let p = (low + (&raw const b) as usize) as *const u64;
    // 42*3 + 1 = 127.
    println!("{}", read_through(p));
}
