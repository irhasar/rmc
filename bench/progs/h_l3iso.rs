fn main() {
    let n = 4_000_000usize;
    let a: Vec<u32> = vec![0x1234_5678; n];
    let b: Vec<u32> = vec![0x9e37_79b1; n];
    let mut c = vec![0u32; n];
    let (pa, pb, pc) = (a.as_ptr(), b.as_ptr(), c.as_mut_ptr());
    let mut acc = 0u32;
    for _r in 0..20 {
        for i in 0..n {
            unsafe { *pc.add(i) = (*pa.add(i) ^ *pb.add(i)).wrapping_mul(2654435761); }
        }
        acc = acc.wrapping_add(unsafe { *pc.add(n - 1) });
    }
    println!("{}", acc);
}
