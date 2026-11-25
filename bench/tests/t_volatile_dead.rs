// A volatile write must survive even when nothing ever reads the destination.
// Rakit has no volatile bit: `write_volatile` lowers to `fence` plus ordinary
// stores, and dead-store elimination used to drop the stores of a 16-byte write
// (the slot looks private and unread once the copy is inlined).
fn main() {
    let mut buf = [0u64; 2];
    unsafe { core::ptr::write_volatile(&mut buf, [0x1111_2222_3333_4444u64, 0x5555_6666_7777_8888]) };
    // No read of `buf`: the only way to observe the write is the code itself.
    println!("written");
}
