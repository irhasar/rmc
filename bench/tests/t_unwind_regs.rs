// Unwinding through a frameless function must reach the frames above it and
// recover the callee-saved registers each of them holds. The x64 prologue used
// to omit RBP while the unwind info still described an RBP-based frame, so a
// backtrace stopped at the first frameless frame.
//
// The check runs through the real unwinder: `_Unwind_Backtrace` must walk
// main -> top -> level -> walk, and `_Unwind_GetGR` on the frames below
// `level` must report the values it keeps live across its call.

#[repr(C)]
struct Ctx {
    _private: [u8; 0],
}

extern "C" {
    fn _Unwind_Backtrace(cb: extern "C" fn(*mut Ctx, *mut u8) -> i32, arg: *mut u8) -> i32;
    fn _Unwind_GetGR(ctx: *mut Ctx, index: i32) -> usize;
    fn _Unwind_GetIP(ctx: *mut Ctx) -> usize;
}

const A: u64 = 0x1111_1111_1111_1111;
const B: u64 = 0x2222_2222_2222_2222;

static mut FRAMES: u32 = 0;
static mut SEEN_A: bool = false;
static mut SEEN_B: bool = false;
static mut SEEN_TOP: bool = false;
static mut TOP_LO: usize = 0;
static mut TOP_HI: usize = 0;

extern "C" fn record(ctx: *mut Ctx, _arg: *mut u8) -> i32 {
    unsafe {
        FRAMES += 1;
        let ip = _Unwind_GetIP(ctx);
        if ip >= TOP_LO && ip < TOP_HI {
            SEEN_TOP = true;
        }
        // rbx, r12, r13, r14, r15 in DWARF numbering.
        for &gr in &[3, 12, 13, 14, 15] {
            let v = _Unwind_GetGR(ctx, gr) as u64;
            if v == A {
                SEEN_A = true;
            }
            if v == B {
                SEEN_B = true;
            }
        }
    }
    0
}

#[inline(never)]
fn walk() {
    unsafe { _Unwind_Backtrace(record, core::ptr::null_mut()) };
}

#[inline(never)]
fn level(a: u64, b: u64) -> u64 {
    // Called indirectly so the inliner keeps the frame, and the arguments stay
    // live across it in callee-saved registers.
    let w: fn() = std::hint::black_box(walk);
    w();
    a ^ b
}

#[inline(never)]
fn top(a: u64, b: u64) -> u64 {
    let l: fn(u64, u64) -> u64 = std::hint::black_box(level);
    l(a, b)
}

fn main() {
    unsafe {
        TOP_LO = top as usize;
        TOP_HI = TOP_LO + 0x400;
    }
    let t: fn(u64, u64) -> u64 = std::hint::black_box(top);
    let r = t(A, B);
    let (frames, a, b, seen_top) = unsafe { (FRAMES, SEEN_A, SEEN_B, SEEN_TOP) };
    // The frame count itself differs between backends; only "the chain went at
    // least main -> top -> level -> walk" is asserted.
    println!("{a} {b} {seen_top} {} {}", frames >= 4, r == (A ^ B));
}
