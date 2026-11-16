#![no_std]
#![no_main]
include!("prelude.rs");

use core::hint::black_box;

// A struct local that is only read and written field by field, or copied
// whole, is replaced by one scalar per field (`scalarize_aggregates`). Each
// shape of whole copy is here: in from an indexed element and out again, in
// and out through a reference, struct to struct, and a literal; plus the uses
// that must keep the struct in memory (its address taken, passed to a call).
// Every field value comes from `arg`, and every field is read back.

#[derive(Clone, Copy)]
struct P {
    x: i64,
    y: i64,
    vx: i64,
    vy: i64,
}

#[inline(never)]
fn sum(p: &P) -> i64 {
    p.x + 3 * p.y + 5 * p.vx + 7 * p.vy
}

#[inline(never)]
fn step(ps: &mut [P; 6], k: i64) {
    let mut i: usize = 0;
    while i < 6 {
        // In from an element, field updates, out to the same element.
        let mut p = ps[i];
        p.vy -= k;
        p.x += p.vx;
        p.y += p.vy;
        if p.x < 0 {
            p.x = -p.x;
            p.vx = -p.vx;
        }
        if p.y > 100 {
            p.y = 200 - p.y;
            p.vy = -p.vy * 9 / 10;
        }
        ps[i] = p;
        i += 1;
    }
}

#[inline(never)]
fn through_ref(dst: &mut P, src: &P, k: i64) {
    // In through one reference, out through another.
    let mut t = *src;
    t.x ^= k;
    t.vy += t.x;
    // Struct to struct, then a literal that reads the copy's fields.
    let u = t;
    let w = P { x: u.vy, y: u.vx, vx: u.y + k, vy: u.x };
    *dst = w;
}

#[inline(never)]
fn kept_in_memory(k: i64) -> i64 {
    // The address of `a` is taken and `b` is passed to a call: neither may
    // be split, and both must still see every field write.
    let mut a = P { x: k, y: k + 1, vx: k + 2, vy: k + 3 };
    let r = &mut a;
    r.y = black_box(40);
    a.vx += a.y;
    let mut b = a;
    b.x = k * 2;
    sum(&b) + a.vx
}

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let k = arg(1);
    let mut ps: [P; 6] = [P { x: 0, y: 0, vx: 0, vy: 0 }; 6];
    let mut i: usize = 0;
    while i < 6 {
        let n = i as i64;
        ps[i] = P { x: k * n - 7, y: n * 30 + k, vx: 5 - n * k, vy: n - k };
        i += 1;
    }
    let mut r: i64 = 0;
    while r < 5 {
        step(&mut ps, k + r);
        r += 1;
    }
    let mut j: usize = 0;
    while j < 6 {
        out(sum(&ps[j]));
        j += 1;
    }
    let mut d = P { x: 1, y: 2, vx: 3, vy: 4 };
    through_ref(&mut d, &ps[2], k);
    out(d.x);
    out(d.y);
    out(d.vx);
    out(d.vy);
    out(kept_in_memory(k));
    0
}
