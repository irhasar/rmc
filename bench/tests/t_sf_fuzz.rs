// Differential fuzz for store forwarding. Forwarding a store into a private
// stack slot is only sound when nothing else can observe that slot, so the pass
// must be invisible: this fixture hammers the shapes that made earlier rounds
// wrong (partial stores, dynamic indices, values merged across branches, raw
// pointer aliases, volatile, unions) and folds every result into a hash.
// `regressions.sh` compares the pass on against the pass off
// (`RAKIT_NO_STORE_FWD=1`) and both against LLVM.

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn mix(h: u64, v: u64) -> u64 {
    (h ^ v).wrapping_mul(0x100_0000_01b3)
}

// Writes go through `&mut` borrows of individual fields, reads through the
// whole struct: the aggregate has to be materialised, but the scalars need not.
#[derive(Clone, Copy)]
struct Fields {
    a: u32,
    b: u64,
    c: u8,
    d: i128,
}

#[inline(never)]
fn field_round_trip(r: &mut Rng) -> u64 {
    let mut s = Fields { a: 0, b: 0, c: 0, d: 0 };
    for _ in 0..3 {
        s.a = r.next() as u32;
        s.c = r.next() as u8;
        let borrow = &mut s.b;
        *borrow = r.next();
        let d = &mut s.d;
        *d = (*d).wrapping_add(r.next() as i128);
    }
    let mut h = 0u64;
    h = mix(h, s.a as u64);
    h = mix(h, s.b);
    h = mix(h, s.c as u64);
    h = mix(h, s.d as u64);
    h = mix(h, (s.d >> 64) as u64);
    h
}

// Partial byte stores at runtime offsets, read back as wider integers.
#[inline(never)]
fn partial_bytes(r: &mut Rng) -> u64 {
    let mut buf = [0u8; 16];
    for _ in 0..6 {
        let i = r.below(16) as usize;
        buf[i] = r.next() as u8;
    }
    let lo = u64::from_ne_bytes(buf[0..8].try_into().unwrap());
    let hi = u64::from_ne_bytes(buf[8..16].try_into().unwrap());
    mix(mix(0, lo), hi)
}

// A value stored in both arms and read after the join.
#[inline(never)]
fn branch_merge(r: &mut Rng) -> u64 {
    let mut acc = 0u64;
    for i in 0..4u64 {
        let x = r.next();
        let slot;
        if x & 1 == 0 {
            slot = x.wrapping_mul(3);
            acc = acc.wrapping_add(slot);
        } else {
            slot = x.wrapping_add(11);
            acc ^= slot;
        }
        acc = acc.wrapping_add(slot >> 3);
        acc ^= i;
    }
    acc
}

// Dynamic indices: the pass cannot name the address, so it must stay out.
#[inline(never)]
fn dynamic_index(r: &mut Rng) -> u64 {
    let mut a = [0u64; 4];
    for _ in 0..8 {
        let i = r.below(4) as usize;
        a[i] = r.next();
    }
    let mut h = 0u64;
    for i in 0..4 {
        h = mix(h, a[i]);
    }
    h
}

// Raw pointer aliases: the address escapes, so the slot is not private.
#[inline(never)]
fn alias_ptr(r: &mut Rng) -> u64 {
    let mut x = 0u64;
    let p = &mut x as *mut u64;
    unsafe { *p = r.next() };
    let y = x;
    let mut z = 0u64;
    let q = &mut z as *mut u64 as usize;
    unsafe { *(q as *mut u64) = r.next() };
    mix(mix(0, y), z)
}

#[inline(never)]
fn opaque_read(p: *const u64) -> u64 {
    unsafe { core::ptr::read(p) }
}

// A volatile write must survive. The value is read back through a pointer handed
// to another function, so the address escapes and no pass can forward the write
// into that read: if the write disappeared, the hash would change.
#[inline(never)]
fn volatile_local(r: &mut Rng) -> u64 {
    let mut v = [0u64; 5];
    let x = r.next();
    unsafe { core::ptr::write_volatile(&mut v[2], x) };
    mix(0, opaque_read(&v[2] as *const u64).wrapping_add(1))
}

// Written through one type, read through another.
#[inline(never)]
fn union_bits(r: &mut Rng) -> u64 {
    union Bits {
        wide: u64,
        bytes: [u8; 8],
    }
    let mut b = Bits { wide: 0 };
    b.wide = r.next();
    let mut h = 0u64;
    for byte in unsafe { b.bytes } {
        h = mix(h, byte as u64);
    }
    h
}

// MaybeUninit: a slot written once and then read as initialised.
#[inline(never)]
fn maybe_uninit(r: &mut Rng) -> u64 {
    let mut m = core::mem::MaybeUninit::<[u64; 2]>::uninit();
    unsafe { m.as_mut_ptr().write([r.next(), r.next()]) };
    let v = unsafe { m.assume_init() };
    mix(mix(0, v[0]), v[1])
}

// Loop-carried state in a struct local, read once after the loop.
#[inline(never)]
fn loop_state(r: &mut Rng) -> u64 {
    struct Acc {
        sum: u64,
        xor: u64,
        count: u32,
    }
    let mut a = Acc { sum: 0, xor: 0, count: 0 };
    let n = 3 + r.below(5);
    for i in 0..n {
        a.sum = a.sum.wrapping_add(i.wrapping_mul(r.next()));
        a.xor ^= i;
        a.count += 1;
    }
    mix(mix(a.sum, a.xor), a.count as u64)
}

// An aggregate assigned in both arms of an `if` and read after the join: the
// stores land in the arms and the loads in the join, which is the shape that
// made the pass miscompile before cross-block forwarding became a must-analysis.
// Four words keeps it in a stack slot; a two-word tuple is split into block
// parameters before code generation and never reaches the pass.
#[inline(never)]
fn cross_block_aggregate(r: &mut Rng) -> u64 {
    let x = r.next();
    let quad: [u64; 4];
    if x & 1 == 0 {
        quad = [x, x.wrapping_add(1), x.wrapping_mul(3), x ^ 7];
    } else {
        quad = [x ^ 1, x.wrapping_add(2), x.wrapping_mul(5), x ^ 11];
    }
    let mut h = 0u64;
    for i in 0..4 {
        h = mix(h, quad[i]);
    }
    h
}

// The same shape inside a loop, where the arm that runs changes per iteration.
// Four words again, so the aggregate stays in a slot and the pass sees stores in
// one block and loads in another.
#[inline(never)]
fn loop_aggregate(r: &mut Rng) -> u64 {
    let mut t = [0u64; 4];
    for i in 0..4u64 {
        if i & 1 == 0 {
            t[0] = t[0].wrapping_add(r.next());
            t[1] ^= r.next();
        } else {
            t[2] = t[2].wrapping_add(r.next());
            t[3] ^= r.next();
        }
        let k = (i & 3) as usize;
        t[k] = t[k].wrapping_add(1);
    }
    mix(mix(t[0], t[1]), mix(t[2], t[3]))
}

fn main() {
    // Runtime input so nothing above folds to a constant.
    let base = std::env::args().count() as u64;
    for n in 0..5u64 {
        let mut r = Rng(base.wrapping_add(n).wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut h = 0u64;
        h = mix(h, field_round_trip(&mut r));
        h = mix(h, partial_bytes(&mut r));
        h = mix(h, branch_merge(&mut r));
        h = mix(h, dynamic_index(&mut r));
        h = mix(h, alias_ptr(&mut r));
        h = mix(h, volatile_local(&mut r));
        h = mix(h, union_bits(&mut r));
        h = mix(h, maybe_uninit(&mut r));
        h = mix(h, loop_state(&mut r));
        h = mix(h, cross_block_aggregate(&mut r));
        h = mix(h, loop_aggregate(&mut r));
        println!("n={n} h={h}");
    }
}
