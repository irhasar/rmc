//! The shape the cross-block dataflow of store forwarding exists for.
//!
//! Both arms store the same values into the same fields and differ only in `h`,
//! so the join has two predecessors that agree on every slot fact: the read after
//! the merge can only be forwarded across the block boundary. `regressions.sh`
//! requires the pass's own `xblk` counter to be positive here and zero with
//! `RAKIT_SF_NO_CBE=1`, which is what makes the count attributable to the shape.

#[inline(never)]
fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x100000001b3)
}

#[inline(never)]
fn cross_block_merge(x: u64) -> u64 {
    let mut buf = [0u64; 4];
    let y = x.wrapping_mul(3);
    let h;
    if x & 1 == 0 {
        buf[0] = y;
        buf[1] = y.wrapping_add(1);
        buf[2] = y ^ 9;
        buf[3] = y << 2;
        h = 1;
    } else {
        buf[0] = y;
        buf[1] = y.wrapping_add(1);
        buf[2] = y ^ 9;
        buf[3] = y << 2;
        h = 2;
    }
    mix(h, buf[0] ^ buf[3])
}

fn main() {
    let mut h = 0u64;
    for i in 0..64u64 {
        h = mix(h, cross_block_merge(i.wrapping_mul(0x9e37_79b9_7f4a_7c15)));
    }
    println!("h={h}");
}
