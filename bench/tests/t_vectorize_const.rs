//! Single-source constant patterns in one address-taken `fn constops`:
//! u32 `a*K` (pmuludq), i64 `a+K` (paddq), f32 `a*K` (mulps), f32 `a-K` (subps).
//! Per-function assertions; outputs must equal LLVM.
#[no_mangle]
pub unsafe extern "C" fn constops(
    p32: *const u32, o32: *mut u32,
    p64: *const u64, o64: *mut u64,
    pf: *const f32, of1: *mut f32, of2: *mut f32,
    n: usize,
) {
    let mut i = 0usize;
    while i < n {
        *o32.add(i) = (*p32.add(i)).wrapping_mul(2654435761);
        i += 1;
    }
    let mut i = 0usize;
    while i < n {
        *o64.add(i) = (*p64.add(i)).wrapping_add(123456789);
        i += 1;
    }
    let mut i = 0usize;
    while i < n {
        *of1.add(i) = *pf.add(i) * 1.5f32;
        i += 1;
    }
    let mut i = 0usize;
    while i < n {
        *of2.add(i) = *pf.add(i) - 0.25f32;
        i += 1;
    }
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
    let a32: Vec<u32> = (0..n).map(|i| (i as u32).wrapping_mul(7) ^ 0x1234).collect();
    let mut o32 = vec![0u32; n];
    let a64: Vec<u64> = (0..n).map(|i| (i as u64).wrapping_mul(7)).collect();
    let mut o64 = vec![0u64; n];
    let af: Vec<f32> = (0..n).map(|i| (i as f32) * 0.5 + 1.25).collect();
    let mut of1 = vec![0f32; n];
    let mut of2 = vec![0f32; n];
    let f: unsafe extern "C" fn(*const u32, *mut u32, *const u64, *mut u64, *const f32, *mut f32, *mut f32, usize) =
        constops;
    unsafe {
        f(a32.as_ptr(), o32.as_mut_ptr(), a64.as_ptr(), o64.as_mut_ptr(), af.as_ptr(), of1.as_mut_ptr(), of2.as_mut_ptr(), n)
    };
    let h32 = ck(&o32.iter().map(|x| *x as u64).collect::<Vec<_>>());
    let h64 = ck(&o64);
    let hf1 = ck(&of1.iter().map(|x| x.to_bits() as u64).collect::<Vec<_>>());
    let hf2 = ck(&of2.iter().map(|x| x.to_bits() as u64).collect::<Vec<_>>());
    println!("u32mul={} i64add={} f32mul={} f32sub={}", h32, h64, hf1, hf2);
}
