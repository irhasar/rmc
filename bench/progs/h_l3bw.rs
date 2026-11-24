fn main() {
    let n = 4_000_000usize;
    let a: Vec<u32> = vec![1u32; n];
    let b: Vec<u32> = vec![2u32; n];
    let mut c = vec![0u32; n];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    let mut acc = 0u32;
    for _r in 0..20 {
        for i in 0..n {
            unsafe { let s = (*pa.add(i)).wrapping_add(*pb.add(i)); *pc.add(i) = s; }
        }
        acc = acc.wrapping_add(unsafe { *pc.add(n - 1) });
    }
    println!("{}", acc);
}
