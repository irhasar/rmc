#![no_std]
#![no_main]
include!("prelude.rs");

// A tuple of mixed widths has one layout for its constant, field addressing,
// element stride and copy length. `(u8, i64)` put its second field at byte 8 but
// measured 9 bytes: in an array every element's `.1` overlapped the next
// element, and `[E; 7]` copied the constant one byte into each field. `(u8, u8)`
// measured 8 with its second field at byte 8. The constants differ in every
// byte (`0x1234_5678_9ABC`), so a field that keeps only some of its bytes, or
// borrows them from a neighbour, cannot match rustc by coincidence (`900` is
// `0x384`, whose high byte is the `3` of `(3, 900)`).
//
// Reads use runtime indices after a runtime write to a different element, so
// rustc cannot fold them through the array.

const E: (u8, i64) = (5, 0x1234_5678_9ABC);
const F: (u8, u8) = (7, 9);

#[inline(never)]
fn wide(k: i64, i: usize) -> i64 {
    let mut a = [E; 7];
    a[(k as usize) % 7].1 = 901;
    a[i % 7].1 + a[(i + 1) % 7].0 as i64
}

#[inline(never)]
fn narrow(k: i64, i: usize) -> i64 {
    let mut a = [F; 5];
    a[(k as usize) % 5].1 = 100;
    let s = a[i % 5].0 as i64 * 1000 + a[i % 5].1 as i64;
    s + a[(i + 1) % 5].1 as i64 * 7
}

struct G {
    v: i64,
}
impl Drop for G {
    fn drop(&mut self) {
        out(self.v);
    }
}

// A droppable field after a narrow one: the glue finds it where the tuple
// keeps it, not one byte in.
#[inline(never)]
fn dropped(k: i64) {
    let t = (k as u8, G { v: k + 40 });
    out(t.0 as i64);
}

#[inline(never)]
fn consume(t: (u8, G)) {
    out(t.0 as i64);
}

#[inline(never)]
fn dropped_arr(k: i64) {
    let a = [(k as u8, G { v: k + 50 }), (k as u8 + 1, G { v: k + 60 })];
    out(a[0].0 as i64 + a[1].0 as i64);
}

// A repeated constant tuple: the copy length must be the tuple's full element
// stride, so an untouched element's `.1` is not truncated by the next element's
// copy. `0x1122334455667788` differs in every byte, so a short copy shows up.
// One element is written at a runtime index and a different one is read at a
// runtime index, so the read cannot be folded to the constant.
#[inline(never)]
fn repeated(k: i64, i: usize) -> i64 {
    let mut a = [(1u8, 0x1122334455667788i64); 4];
    a[(k as usize) % 4].1 = 901;
    a[i % 4].1 + a[(i + 1) % 4].0 as i64
}

// The other field orders: `(i64, u8)` keeps `.0` in the first word and `.1`
// in the second, and `(u8, u8)` keeps both bytes at 0 and 8 of its 16-byte
// slot. A copy shorter than the slot drops the second field.
#[inline(never)]
fn repeated_i64_u8(k: i64, i: usize) -> i64 {
    let mut a = [(0x1122334455667788i64, 0x9Au8); 4];
    a[(k as usize) % 4].0 = 7;
    a[i % 4].0 + a[(i + 1) % 4].1 as i64
}

#[inline(never)]
fn repeated_u8_u8(k: i64, i: usize) -> i64 {
    let mut a = [(0xABu8, 0xCDu8); 4];
    a[(k as usize) % 4].1 = 0x11;
    a[i % 4].1 as i64 * 1000 + a[(i + 1) % 4].0 as i64
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let i = arg(2) as usize;
    out(repeated(k, i));
    out(repeated_i64_u8(k, i));
    out(repeated_u8_u8(k, i));
    out(wide(k, i));
    out(narrow(k, i));
    dropped(k);
    consume((k as u8, G { v: k + 45 }));
    dropped_arr(k);
    0
}
