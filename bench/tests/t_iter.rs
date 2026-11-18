fn main() {
    let mut acc: u64 = 0;
    for i in 0..100000u64 { acc = acc.wrapping_add(i.wrapping_mul(i)); }
    let v: Vec<u64> = (0..50000u64).filter(|x| x%3==0).map(|x| x*2).collect();
    let s: u64 = v.iter().copied().fold(0, u64::wrapping_add);
    let zipped: u64 = (0..10000u64).zip((0..10000u64).rev()).map(|(a,b)| a.wrapping_mul(b)).fold(0,u64::wrapping_add);
    let chain: u64 = (0..1000u64).chain(1000..2000).step_by(7).fold(0, u64::wrapping_add);
    println!("{} {} {} {}", acc, s, zipped, chain);
}
