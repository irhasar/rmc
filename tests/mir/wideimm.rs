#![no_std]
#![no_main]
include!("prelude.rs");

// A 64-bit constant is not encodable as an ALU immediate: `add $imm32, %rax`
// sign-extends its i32 field, so the back end must materialise the value in a
// register first. These cases pin that, plus the same truncation on a frame
// store and on a `switchInt` discriminant.
const WIDE: i64 = 0x4000_0000_0000_0001; // 4611686018427387905
const MED: i64 = 0x1_0000_0001; // 4294967297, just over i32

#[inline(never)]
fn id(x: i64) -> i64 {
    x
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    out(WIDE + 1);
    out(WIDE - 1);
    out(WIDE ^ 3);
    out(WIDE & WIDE);
    out(WIDE | 2);
    out(1 - WIDE);
    out(3 * MED);

    let mut x: i64 = 0;
    x += WIDE; // a wide immediate stored into a frame slot
    out(x);
    x -= WIDE;
    out(x);

    // A wide `switchInt` value: `cmp $imm, reg` would truncate it.
    let m = match WIDE + 1 {
        WIDE => 111,
        _ => 222,
    };
    out(m);

    // Comparisons against a wide immediate, on either side.
    out((WIDE < 0) as i64);
    out((WIDE == MED) as i64);
    out((MED == WIDE) as i64);

    // A shift whose count is computed by a call: the value must survive the
    // call (which clobbers the scratch registers) in a frame slot.
    out(1 << (id(MED) & 63));
    out((WIDE >> 8) & 0xff);
    0
}
