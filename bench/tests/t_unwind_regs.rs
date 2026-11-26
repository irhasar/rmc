// Unwinding through a frameless function must reach the frames above it and
// recover the callee-saved registers they hold. The x64 prologue used to omit
// RBP while the unwind info still described an RBP-based frame, so a backtrace
// stopped at the first frameless frame.
//
// The chain is caller -> mid -> walk. The caller holds two magic values in
// callee-saved registers across its call, `mid` overwrites those registers with
// four magic values of its own (so its prologue has to save the caller's
// copies), and `walk` starts the unwind through the real unwinder. A frame is
// accepted as the caller only if it reports both of the caller's values and
// none of `mid`'s, so values that merely happen to still sit in registers
// cannot satisfy the check.

#[repr(C)]
struct Ctx {
    _private: [u8; 0],
}

extern "C" {
    fn _Unwind_Backtrace(cb: extern "C" fn(*mut Ctx, *mut u8) -> i32, arg: *mut u8) -> i32;
    fn _Unwind_GetGR(ctx: *mut Ctx, index: i32) -> usize;
}

const A: u64 = 0x1111_1111_1111_1111;
const B: u64 = 0x2222_2222_2222_2222;
const C: u64 = 0x3333_3333_3333_3333;
const D: u64 = 0x4444_4444_4444_4444;
const E: u64 = 0x5555_5555_5555_5555;
const F: u64 = 0x6666_6666_6666_6666;

static mut FRAMES: u32 = 0;
static mut SEEN_CALLER: bool = false;
static mut SEEN_CLOBBER: bool = false;

extern "C" fn record(ctx: *mut Ctx, _arg: *mut u8) -> i32 {
    unsafe {
        FRAMES += 1;
        // rbx, r12, r13, r14, r15 in DWARF numbering.
        let mut regs = [0u64; 5];
        for (i, &gr) in [3, 12, 13, 14, 15].iter().enumerate() {
            regs[i] = _Unwind_GetGR(ctx, gr) as u64;
        }
        let caller_values = regs.iter().filter(|&&v| v == A || v == B).count();
        let mid_values = regs
            .iter()
            .filter(|&&v| v == C || v == D || v == E || v == F)
            .count();
        if caller_values == 2 && mid_values == 0 {
            SEEN_CALLER = true;
        }
        if mid_values >= 2 {
            SEEN_CLOBBER = true;
        }
    }
    0
}

#[inline(never)]
fn walk() {
    unsafe { _Unwind_Backtrace(record, core::ptr::null_mut()) };
}

#[inline(never)]
fn mid(c: u64, d: u64, e: u64, f: u64) -> u64 {
    // All four stay live across the call, so the allocator takes callee-saved
    // registers and the prologue has to save the caller's copies of A and B.
    let w: fn() = std::hint::black_box(walk);
    w();
    c ^ d.rotate_left(1) ^ e.rotate_left(2) ^ f.rotate_left(3)
}

fn caller(a: u64, b: u64) -> u64 {
    let m: fn(u64, u64, u64, u64) -> u64 = std::hint::black_box(mid);
    let r = m(C, D, E, F);
    // `a` and `b` are needed after the call, so they live in callee-saved
    // registers across it.
    a ^ b ^ r
}

fn main() {
    let t: fn(u64, u64) -> u64 = std::hint::black_box(caller);
    let r = t(A, B);
    let (frames, seen_caller, seen_clobber) = unsafe { (FRAMES, SEEN_CALLER, SEEN_CLOBBER) };
    let expected = A ^ B ^ C ^ D.rotate_left(1) ^ E.rotate_left(2) ^ F.rotate_left(3);
    // The frame count itself differs between backends; only "the unwind went at
    // least caller -> mid -> walk" is asserted.
    println!("{seen_caller} {seen_clobber} {} {}", frames >= 4, r == expected);
}
