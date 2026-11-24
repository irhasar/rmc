//! Non-zero loop start: `for i in s..n` for a map (`startmul` -> pmuludq) and a
//! sum reduction (`startsum` -> paddd), s a runtime value. The vector loop must
//! begin at s and the scalar tail cover [s + k*per_vec, n). Exercises s=0/1/3/5
//! and s==n (empty). Outputs must equal LLVM.
#[no_mangle]
pub unsafe extern "C" fn startmul(pa: *const u32, pc: *mut u32, s: usize, n: usize) {
    let mut i = s;
    while i < n {
        *pc.add(i) = (*pa.add(i)).wrapping_mul(2654435761);
        i += 1;
    }
}
#[no_mangle]
pub unsafe extern "C" fn startsum(pa: *const u32, init: u32, s: usize, n: usize) -> u32 {
    let mut acc = init;
    let mut i = s;
    while i < n {
        acc = acc.wrapping_add(*pa.add(i));
        i += 1;
    }
    acc
}
fn ck(v: &[u64]) -> u64 {
    let mut h = 1469598103934665603u64;
    for &x in v {
        h = (h ^ x).wrapping_mul(1099511628211);
    }
    h
}
fn main() {
    let n = std::hint::black_box(1000usize);
    let a: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let f: unsafe extern "C" fn(*const u32, *mut u32, usize, usize) = startmul;
    let g: unsafe extern "C" fn(*const u32, u32, usize, usize) -> u32 = startsum;
    for &raw in &[0usize, 1, 3, 5, 1000] {
        let s = std::hint::black_box(raw);
        let mut c = vec![0u32; n];
        unsafe { f(a.as_ptr(), c.as_mut_ptr(), s, n) };
        let hs = ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>());
        let ss = unsafe { g(a.as_ptr(), 12345, s, n) };
        println!("s={} mul={} sum={}", s, hs, ss);
    }
    // A start near usize::MAX must not touch memory: the vector header has to
    // reject the wrapped `i + per_vec` instead of taking it for a fit.
    let mut c = vec![0xAAAA_AAAAu32; 8];
    unsafe { f(a.as_ptr().add(1), c.as_mut_ptr().add(1), usize::MAX, 3) };
    let sum = unsafe { g(a.as_ptr().add(1), 12345, usize::MAX, 3) };
    println!(
        "wrap mul={} sum={}",
        ck(&c.iter().map(|x| *x as u64).collect::<Vec<_>>()),
        sum
    );
}
