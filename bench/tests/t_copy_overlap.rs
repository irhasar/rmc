// `ptr::copy` may overlap: it must behave like a copy through a temporary.
// Constant sizes 16..64 used to take the vector-chunk path, which stores each
// chunk before loading the next and therefore duplicated bytes.
use std::ptr;

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(1469598103934665603u64, |h, &b| (h ^ b as u64).wrapping_mul(1099511628211))
}

fn run<const N: usize>(shift: usize) {
    // Dest one byte ahead: the first store lands on the second chunk's source.
    let mut buf = [0u8; 128];
    for (i, b) in buf.iter_mut().enumerate() {
        *b = i as u8;
    }
    unsafe { ptr::copy(buf.as_ptr(), buf.as_mut_ptr().add(shift), N) };
    let mut tmp = [0u8; 128];
    tmp.copy_from_slice(&buf);
    println!("N={N} shift={shift} {}", hash(&tmp[..shift + N]));
    // ... and one byte behind.
    let mut back = [0u8; 128];
    for (i, b) in back.iter_mut().enumerate() {
        *b = (255 - i) as u8;
    }
    unsafe { ptr::copy(back.as_ptr().add(shift), back.as_mut_ptr(), N) };
    let mut tmp2 = [0u8; 128];
    tmp2.copy_from_slice(&back);
    println!("back N={N} shift={shift} {}", hash(&tmp2[..shift + N]));
}

fn main() {
    run::<16>(1);
    run::<32>(1);
    run::<32>(7);
    run::<48>(3);
    run::<64>(1);
    run::<64>(16);
    // Non-overlapping still has to be exact.
    let mut dst = [0u8; 64];
    let src = [9u8; 64];
    unsafe { ptr::copy(src.as_ptr(), dst.as_mut_ptr(), 64) };
    println!("nonoverlap {}", hash(&dst));
}
