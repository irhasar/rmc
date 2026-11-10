// The Rust twin of `listptr.rms`: bump through one arena, write each node's
// fields through a raw pointer, then chase the links. Same pointer arithmetic
// as the rmc program, so the printed sum must match.
struct Node {
    val: i64,
    next: i64,
}

fn main() {
    let n: i64 = 5000;
    let mut arena: Vec<u8> = vec![0u8; n as usize * 16];
    let base = arena.as_mut_ptr();

    let mut head: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        let p = unsafe { base.add(i as usize * 16) } as *mut Node;
        unsafe {
            (*p).val = i;
            (*p).next = head;
        }
        head = p as i64;
        i += 1;
    }

    let mut sum: i64 = 0;
    let mut cur: i64 = head;
    while cur != 0 {
        let cp = cur as *const Node;
        sum += unsafe { (*cp).val };
        cur = unsafe { (*cp).next };
    }
    println!("{}", sum);
}
