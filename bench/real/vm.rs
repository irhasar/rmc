// A `bench/real` workload: realistic code that was *not* used to tune rmc,
// so it checks whether a speed-up seen on `bench/uni` carries over (see
// `bench/run_four.py --set real`). The same program is a regression case in
// `tests/mir/real_*.rs`. The prelude below is the shared `bench/uni` one.

#![no_std]
#![no_main]

// One source, four back ends: rmc's AST front end, and rustc driving rmc's
// codegen backend, rustc's Cranelift backend, and rustc's LLVM backend.
//
// The runtime declarations and the panic handler live in `mod rmc_rt`, and the
// names are re-exported with `use`. rmc drops `mod` bodies and `use` imports
// and resolves these names as builtins; rustc compiles the module and imports
// them. Both see the same calls, which is what lets one file drive all four.
mod rmc_rt {
    extern "C" {
        pub fn print_i64(x: i64);
        pub fn arg_i64(i: i64) -> i64;
        pub fn alloc(n: i64) -> i64;
        pub fn free(p: i64);
        pub fn putchar(c: i64);
        pub fn exit(code: i64) -> !;
    }
    use core::panic::PanicInfo;
    #[panic_handler]
    pub fn panic(_info: &PanicInfo) -> ! {
        unsafe { exit(101) }
    }
}
use rmc_rt::print_i64;
use rmc_rt::arg_i64;
use rmc_rt::alloc;
use rmc_rt::free;
use rmc_rt::putchar;

#[inline(always)]
fn print(x: i64) {
    unsafe { print_i64(x) }
}

#[inline(always)]
fn arg(i: i64) -> i64 {
    unsafe { arg_i64(i) }
}

#[inline(always)]
fn alc(n: i64) -> i64 {
    unsafe { alloc(n) }
}

fn emit(x: i64) {
    print(x)
}

// Bytecode interpreter: a stack VM running a Collatz program.
// Real-world shape: the opcode dispatch loop of a scripting-language VM.
const PUSH: i64 = 0;
const LOAD: i64 = 1;
const STORE: i64 = 2;
const ADD: i64 = 3;
const SUB: i64 = 4;
const MUL: i64 = 5;
const REM: i64 = 6;
const DIV: i64 = 7;
const LT: i64 = 8;
const EQ: i64 = 9;
const JZ: i64 = 10;
const JMP: i64 = 11;
const LEN: usize = 40;

fn interp(ops: &[i64; LEN], args: &[i64; LEN], n: i64) -> i64 {
    let mut stack: [i64; 32] = [0; 32];
    let mut locals: [i64; 8] = [0; 8];
    locals[4] = n;
    let mut pc: i64 = 0;
    let mut sp: i64 = 0;
    loop {
        let op = ops[pc as usize];
        let a = args[pc as usize];
        pc += 1;
        match op {
            PUSH => {
                stack[sp as usize] = a;
                sp += 1;
            }
            LOAD => {
                stack[sp as usize] = locals[a as usize];
                sp += 1;
            }
            STORE => {
                sp -= 1;
                locals[a as usize] = stack[sp as usize];
            }
            ADD => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = stack[t].wrapping_add(b);
            }
            SUB => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = stack[t].wrapping_sub(b);
            }
            MUL => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = stack[t].wrapping_mul(b);
            }
            REM => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = stack[t] % b;
            }
            DIV => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = stack[t] / b;
            }
            LT => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = if stack[t] < b { 1 } else { 0 };
            }
            EQ => {
                sp -= 1;
                let b = stack[sp as usize];
                let t = (sp - 1) as usize;
                stack[t] = if stack[t] == b { 1 } else { 0 };
            }
            JZ => {
                sp -= 1;
                if stack[sp as usize] == 0 {
                    pc = a;
                }
            }
            JMP => {
                pc = a;
            }
            _ => break,
        }
    }
    locals[2]
}

fn run(n: i64) {
    let n = if n == 0 { 300000 } else { n };
    // (op, arg) pairs. Source program:
    // for i in 1..n { x = i; while x != 1 { x = if x % 2 == 0 { x / 2 } else { 3x + 1 }; steps += 1 } }
    let prog: [i64; 80] = [
        PUSH, 1, STORE, 0,
        LOAD, 0, LOAD, 4, LT, 0, JZ, 39,
        LOAD, 0, STORE, 1,
        LOAD, 1, PUSH, 1, EQ, 0, PUSH, 0, EQ, 0, JZ, 34,
        LOAD, 1, PUSH, 2, REM, 0, JZ, 25,
        LOAD, 1, PUSH, 3, MUL, 0, PUSH, 1, ADD, 0, STORE, 1, JMP, 29,
        LOAD, 1, PUSH, 2, DIV, 0, STORE, 1,
        LOAD, 2, PUSH, 1, ADD, 0, STORE, 2, JMP, 8,
        LOAD, 0, PUSH, 1, ADD, 0, STORE, 0, JMP, 2,
        99, 0,
    ];
    let mut ops: [i64; LEN] = [0; LEN];
    let mut args: [i64; LEN] = [0; LEN];
    let mut i: usize = 0;
    while i < LEN {
        ops[i] = prog[2 * i];
        args[i] = prog[2 * i + 1];
        i += 1;
    }
    emit(interp(&ops, &args, n));
}

#[no_mangle]
pub fn main() -> i64 {
    run(arg(1));
    0
}
