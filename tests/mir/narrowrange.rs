#![no_std]
#![no_main]
include!("prelude.rs");

// Arithmetic on a narrow signed type wraps inside its own width: in an `i8`,
// `126 + 2` is -128, not 128. The bounds-check eliminator tracks an upper bound
// for `(x & 126) + 2` and must not carry it across the wrap: the cast to `usize`
// of -128 is a huge index, and the check against the array length has to stay.
//
// The first argument picks the shape and the second is the runtime value, so
// each run exercises one wrapping or non-wrapping input. A wrapping input must
// make both binaries exit 101 (the panic handler); a non-wrapping one must read
// the same element on both.
//
// The `u8` shape is the control: an unsigned result that wraps is still at most
// the bound that was tracked, so its check may go, and it must still agree.

#[inline(never)]
fn i8_add(t: &[u8; 129], x: i8) -> u8 {
    t[(x & 126_i8).wrapping_add(2) as usize]
}

#[inline(never)]
fn i8_mul(t: &[u8; 140], x: i8) -> u8 {
    t[(x & 15_i8).wrapping_mul(9) as usize]
}

#[inline(never)]
fn i8_shl(t: &[u8; 256], x: i8) -> u8 {
    t[((x & 63_i8) << 2) as usize]
}

#[inline(never)]
fn i16_add(t: &[u8; 32770], x: i16) -> u8 {
    t[(x & 0x7FFE_i16).wrapping_add(2) as usize]
}

#[inline(never)]
fn u8_add(t: &[u8; 300], x: u8) -> u8 {
    t[(x & 0xF0_u8).wrapping_add(0x20) as usize]
}

fn fill<const N: usize>(t: &mut [u8; N]) {
    let mut i = 0;
    while i < N {
        t[i] = (i * 7 + 3) as u8;
        i += 1;
    }
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let shape = arg(1);
    let v = arg(2);
    let r = if shape == 0 {
        let mut t = [0u8; 129];
        fill(&mut t);
        i8_add(&t, v as i8)
    } else if shape == 1 {
        let mut t = [0u8; 140];
        fill(&mut t);
        i8_mul(&t, v as i8)
    } else if shape == 2 {
        let mut t = [0u8; 256];
        fill(&mut t);
        i8_shl(&t, v as i8)
    } else if shape == 3 {
        let mut t = [0u8; 32770];
        fill(&mut t);
        i16_add(&t, v as i16)
    } else {
        let mut t = [0u8; 300];
        fill(&mut t);
        u8_add(&t, v as u8)
    };
    out(r as i64);
    0
}
