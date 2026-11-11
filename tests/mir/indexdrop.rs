#![no_std]
#![no_main]
include!("prelude.rs");

// Dropping `arr[i]` with a runtime index lowers to `drop(_arr[_i])`, and the
// back end materialises the element address by reading the index local. If `_i`
// is promoted to a caller-saved register, that register is clobbered by the
// destructor calls the glue itself makes (`drop_in_place` of the array field,
// then of `b`). The glue is emitted under one `spill_caller_saved` at its
// start, so a read *inside* the tree must take the value from the spill slot,
// not the stale register -- otherwise `b` is dropped at `arr[<garbage>]`.
//
// The nested loops are load-bearing: they raise `_i`'s use count enough for the
// register allocator to promote it into a caller-saved register, which is what
// makes the clobber reachable. Without them `_i` stays in its frame slot and
// the bug is latent.

struct Inner { v: i64 }
impl Drop for Inner {
    fn drop(&mut self) { out(self.v); }
}
struct S { a: [Inner; 2], b: Inner }

#[no_mangle]
pub extern "C" fn main() -> i64 {
    let n = arg(1);
    let i = (arg(2) & 1) as usize;
    let mut s = 0i64;
    for a in 0..n {
        for b in 0..n {
            for c in 0..n {
                for d in 0..n {
                    for e in 0..n {
                        s += a + b + c + d + e + i as i64;
                    }
                }
            }
        }
    }
    let mut arr = [S { a: [Inner { v: 11 }, Inner { v: 12 }], b: Inner { v: 13 } },
                   S { a: [Inner { v: 21 }, Inner { v: 22 }], b: Inner { v: 23 } }];
    arr[i] = S { a: [Inner { v: 91 }, Inner { v: 92 }], b: Inner { v: 93 } };
    out(s + arr[0].b.v + arr[1].b.v);
    0
}
