// A volatile write must survive even when nothing ever reads the destination.
// Rakit has no volatile bit: `write_volatile` lowers to `fence` plus ordinary
// stores, and dead-store elimination used to drop the stores of a 16-byte write
// (the slot looks private and unread once the copy is inlined).
#[repr(C)]
#[derive(Clone, Copy)]
struct S40 {
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
}

// The source values live in .rodata, so building them costs no stack stores: the
// stores of each fenced region are the write's own copy, which is what
// `bench/regressions.sh` counts.
static SRC16: [u64; 2] = [0x1111_2222_3333_4444, 0x5555_6666_7777_8888];
static SRC40: S40 = S40 { a: 9, b: 8, c: 7, d: 6, e: 5 };

fn main() {
    let mut buf = [0u64; 2];
    // 40 bytes: not a multiple of 16, so the write is a memcpy the pass inlines
    // into 16+16+8 stores placed *after* the fence.
    let mut s = S40 { a: 1, b: 2, c: 3, d: 4, e: 5 };
    unsafe { core::ptr::write_volatile(&mut buf, SRC16) };
    unsafe { core::ptr::write_volatile(&mut s, SRC40) };
    // No read of either destination: the only way to observe a write is the code
    // itself.
    println!("written");
}
