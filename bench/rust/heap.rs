// The Rust twin of `heap.rms`: build a linked list by bumping through one
// arena, then walk it. A bump arena (not `Box`/`malloc`) keeps the comparison
// about the generated code — the same pointer arithmetic and loads — rather
// than about rmc's free-list allocator versus glibc's.
struct Node {
    val: i64,
    next: i64,
}

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let n = std::hint::black_box(n);
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

    // Pointer chasing: each step depends on the previous load, so the walk
    // cannot be folded into a closed form the way a plain sum could.
    let mut sum: i64 = 0;
    let mut cur: i64 = head;
    while cur != 0 {
        let cp = cur as *const Node;
        sum += unsafe { std::hint::black_box((*cp).val) };
        cur = unsafe { (*cp).next };
    }
    println!("{}", sum);
}
