// The Rust twin of `bench/mir/heap.rs`: build a linked list by bumping through one
// arena, then walk it. A bump arena (not `Box`/`malloc`) keeps the comparison
// about the generated code — the same pointer arithmetic and loads — rather
// than about rmc's free-list allocator versus glibc's.
//
// rmc's `alloc` hands back a payload that sits 8 bytes past a size header, so
// consecutive 16-byte nodes are 24 bytes apart, not 16. The twin uses the same
// 24-byte stride (node at `base + 8 + i*24`) so both walks touch the same
// number of cache lines; packing the Rust nodes 16 bytes apart would give its
// walk a smaller working set and flatter the comparison.
struct Node {
    val: i64,
    next: i64,
}

const STRIDE: usize = 24;

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let n = std::hint::black_box(n);
    let mut arena: Vec<u8> = vec![0u8; n as usize * STRIDE];
    let base = arena.as_mut_ptr();

    let mut head: i64 = 0;
    let mut i: i64 = 0;
    while i < n {
        // Mirror rmc: the node's payload starts 8 bytes past the slot start.
        let p = unsafe { base.add(i as usize * STRIDE + 8) } as *mut Node;
        unsafe {
            (*p).val = i;
            (*p).next = head;
        }
        head = p as i64;
        i += 1;
    }

    // Pointer chasing: each step depends on the previous load, so the walk
    // cannot be folded into a closed form the way a plain sum could, and no
    // in-loop `black_box` is needed to keep it real. That matters for fairness:
    // rmc lowers `black_box` to the identity while rustc makes it opaque, so an
    // in-loop one would charge rustc a store/reload rmc never emits.
    let mut sum: i64 = 0;
    let mut cur: i64 = head;
    while cur != 0 {
        let cp = cur as *const Node;
        sum += unsafe { (*cp).val };
        cur = unsafe { (*cp).next };
    }
    println!("{}", sum);
}
