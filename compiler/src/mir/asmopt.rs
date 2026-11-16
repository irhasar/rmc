//! Register-level clean-up of one function's generated assembly.
//!
//! The lowering in `codegen.rs` computes every value in a scratch register and
//! then moves it to its home, and the text peepholes that follow only know a
//! register is free when it is one of the scratch registers. Once most values
//! live in registers of their own, what is left is chains of moves between
//! registers that nothing can remove without knowing which registers are
//! still needed afterwards.
//!
//! This pass parses the body into instructions with explicit use and
//! definition sets, computes register liveness over the function's control
//! flow, and then:
//!
//! * **propagates copies** forward: after `mov %a, %b`, a read of `%b` reads
//!   `%a` instead, for as long as neither is written;
//! * **forwards addresses**: after `lea M, %t`, a memory operand `k(%t)` is
//!   rewritten to `M + k`;
//! * **retargets** a value computed in a scratch register and then moved
//!   (`mov M, %t; mov %t, %d`) to be computed in its destination;
//! * **deletes** every move, load and `lea` whose result is dead.
//!
//! It is all-or-nothing per function: an instruction the parser does not model
//! leaves the function's text exactly as it was. Nothing here changes flags
//! (only flag-neutral instructions are rewritten or removed), and nothing
//! moves a memory access across another.

use std::collections::{HashMap, HashSet};

type RegSet = u32;

const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
const RSP: u8 = 4;
const RBP: u8 = 5;
const RSI: u8 = 6;
const RDI: u8 = 7;

const fn bit(r: u8) -> RegSet {
    1 << r
}

/// Registers a call may read: the six argument registers, `%rax` (the
/// variadic count, and an indirect callee is often there) and `%r10`/`%r11`,
/// which the runtime's internal helpers take arguments in.
const CALL_USES: RegSet =
    bit(RDI) | bit(RSI) | bit(RDX) | bit(RCX) | bit(8) | bit(9) | bit(RAX) | bit(10) | bit(11);
/// The integer argument registers, in order.
const ARG_ORDER: [u8; 6] = [RDI, RSI, RDX, RCX, 8, 9];
/// Registers a call may overwrite.
const CALL_CLOBBERS: RegSet =
    bit(RAX) | bit(RCX) | bit(RDX) | bit(RSI) | bit(RDI) | bit(8) | bit(9) | bit(10) | bit(11);
/// Live at a `ret`: the return value, and everything the caller expects back
/// (the callee-saved registers, which the epilogue's `pop`s have restored).
const RET_USES: RegSet = bit(RAX) | bit(RDX) | bit(3) | bit(RSP) | bit(RBP) | bit(12) | bit(13) | bit(14) | bit(15);
/// Live when a body given without its epilogue falls off its end or jumps to
/// `.Lret_*`: the same set, conservatively.
const EXIT_LIVE: RegSet = RET_USES;
/// Never renamed, never treated as dead.
const FIXED: RegSet = bit(RSP) | bit(RBP);

const NAMES64: [&str; 16] = [
    "%rax", "%rcx", "%rdx", "%rbx", "%rsp", "%rbp", "%rsi", "%rdi", "%r8", "%r9", "%r10", "%r11",
    "%r12", "%r13", "%r14", "%r15",
];
const NAMES32: [&str; 16] = [
    "%eax", "%ecx", "%edx", "%ebx", "%esp", "%ebp", "%esi", "%edi", "%r8d", "%r9d", "%r10d",
    "%r11d", "%r12d", "%r13d", "%r14d", "%r15d",
];
const NAMES16: [&str; 16] = [
    "%ax", "%cx", "%dx", "%bx", "%sp", "%bp", "%si", "%di", "%r8w", "%r9w", "%r10w", "%r11w",
    "%r12w", "%r13w", "%r14w", "%r15w",
];
const NAMES8: [&str; 16] = [
    "%al", "%cl", "%dl", "%bl", "%spl", "%bpl", "%sil", "%dil", "%r8b", "%r9b", "%r10b", "%r11b",
    "%r12b", "%r13b", "%r14b", "%r15b",
];

/// A general-purpose register name: its number and its width in bytes.
fn parse_reg(s: &str) -> Option<(u8, u8)> {
    for (names, w) in [(&NAMES64, 8u8), (&NAMES32, 4), (&NAMES16, 2), (&NAMES8, 1)] {
        if let Some(i) = names.iter().position(|n| *n == s) {
            return Some((i as u8, w));
        }
    }
    None
}

fn reg_name(r: u8, w: u8) -> &'static str {
    match w {
        8 => NAMES64[r as usize],
        4 => NAMES32[r as usize],
        2 => NAMES16[r as usize],
        _ => NAMES8[r as usize],
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Mem {
    /// The displacement: a number, or a symbol expression when `rip` is set.
    disp: String,
    base: Option<u8>,
    index: Option<u8>,
    scale: u8,
    /// `sym(%rip)`: reads no general-purpose register.
    rip: bool,
}

impl Mem {
    fn regs(&self) -> RegSet {
        self.base.map_or(0, bit) | self.index.map_or(0, bit)
    }

    fn num_disp(&self) -> Option<i64> {
        if self.rip {
            return None;
        }
        if self.disp.is_empty() {
            Some(0)
        } else {
            self.disp.parse().ok()
        }
    }

    fn text(&self) -> String {
        if self.rip {
            return format!("{}(%rip)", self.disp);
        }
        let b = self.base.map_or("", |r| NAMES64[r as usize]);
        match self.index {
            None => format!("{}({b})", self.disp),
            Some(i) => format!("{}({b},{},{})", self.disp, NAMES64[i as usize], self.scale),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Opnd {
    Reg(u8, u8),
    Imm(String),
    Mem(Mem),
    /// A label, a symbol, an `%xmm` register, an indirect-call target.
    Other(String),
}

impl Opnd {
    fn text(&self) -> String {
        match self {
            Opnd::Reg(r, w) => reg_name(*r, *w).to_string(),
            Opnd::Imm(s) | Opnd::Other(s) => s.clone(),
            Opnd::Mem(m) => m.text(),
        }
    }
}

fn parse_mem(s: &str) -> Option<Mem> {
    let open = s.find('(')?;
    if !s.ends_with(')') {
        return None;
    }
    let disp = s[..open].trim().to_string();
    let inner = &s[open + 1..s.len() - 1];
    let parts: Vec<&str> = inner.split(',').map(|p| p.trim()).collect();
    if parts.len() == 1 && parts[0] == "%rip" {
        return Some(Mem { disp, base: None, index: None, scale: 1, rip: true });
    }
    if parts.len() > 3 {
        return None;
    }
    let reg64 = |p: &str| match parse_reg(p) {
        Some((r, 8)) => Some(r),
        _ => None,
    };
    let base = if parts[0].is_empty() { None } else { Some(reg64(parts[0])?) };
    let index = match parts.get(1) {
        Some(p) if !p.is_empty() => Some(reg64(p)?),
        _ => None,
    };
    let scale = match parts.get(2) {
        Some(p) => p.parse::<u8>().ok().filter(|s| matches!(s, 1 | 2 | 4 | 8))?,
        None => 1,
    };
    // A displacement must be a plain number: anything else (a symbol without
    // `%rip`) is not something this pass can add to.
    if !disp.is_empty() && disp.parse::<i64>().is_err() {
        return None;
    }
    Some(Mem { disp, base, index, scale, rip: false })
}

fn parse_opnd(s: &str) -> Opnd {
    let s = s.trim();
    if let Some((r, w)) = parse_reg(s) {
        return Opnd::Reg(r, w);
    }
    if s.starts_with('$') {
        return Opnd::Imm(s.to_string());
    }
    if s.contains('(') && !s.starts_with('*') {
        if let Some(m) = parse_mem(s) {
            return Opnd::Mem(m);
        }
    }
    Opnd::Other(s.to_string())
}

/// Split an operand list at the commas outside parentheses.
fn split_operands(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Flow {
    Next,
    /// Unconditional jump to the label with this index in `Func::labels`.
    Jmp,
    Jcc,
    /// Never continues (`ud2`, `hlt`).
    Stop,
}

/// What kind of line this is, as far as the rewrites care.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Label,
    Directive,
    /// `mov`/`movq`/`movabs` between 64- or 32-bit operands.
    Mov,
    Lea,
    /// Any other instruction.
    Insn,
}

#[derive(Clone, Debug)]
struct Inst {
    text: String,
    kind: Kind,
    mnem: String,
    ops: Vec<Opnd>,
    /// For each operand, whether a register operand there is only read (so a
    /// copy may be substituted). Memory operands' address registers are
    /// always only read.
    read_only: Vec<bool>,
    uses: RegSet,
    /// Registers fully overwritten without the old value mattering.
    kills: RegSet,
    /// Every register written, fully, partially, or clobbered.
    writes: RegSet,
    /// Registers the instruction needs by name without spelling them as a
    /// renamable operand (`%cl` shift counts, `%rax`/`%rdx` of a division).
    implicit: RegSet,
    reads_mem: bool,
    writes_mem: bool,
    flow: Flow,
    target: Option<String>,
    /// The instruction does nothing but define this register, and does not
    /// touch the flags: it can go when the register is dead.
    pure_def: Option<u8>,
    is_call: bool,
    dead: bool,
}

impl Inst {
    fn blank(text: &str, kind: Kind) -> Inst {
        Inst {
            text: text.to_string(),
            kind,
            mnem: String::new(),
            ops: Vec::new(),
            read_only: Vec::new(),
            uses: 0,
            kills: 0,
            writes: 0,
            implicit: 0,
            reads_mem: false,
            writes_mem: false,
            flow: Flow::Next,
            target: None,
            pure_def: None,
            is_call: false,
            dead: false,
        }
    }

    fn render(&mut self) {
        let ops: Vec<String> = self.ops.iter().map(|o| o.text()).collect();
        self.text = if ops.is_empty() {
            format!("  {}", self.mnem)
        } else {
            format!("  {} {}", self.mnem, ops.join(", "))
        };
    }
}

/// Strip an operand-size suffix from a mnemonic this pass knows by its base
/// name (`addq` -> `add`).
fn base_mnem(m: &str) -> &str {
    const SUFFIXED: &[&str] = &[
        "mov", "add", "sub", "and", "or", "xor", "cmp", "test", "adc", "sbb", "neg", "not", "inc",
        "dec", "sal", "shl", "sar", "shr", "rol", "ror", "imul", "idiv", "div", "mul", "lea",
        "push", "pop",
    ];
    for suf in ['q', 'l', 'b', 'w'] {
        if let Some(b) = m.strip_suffix(suf) {
            if SUFFIXED.contains(&b) {
                return b;
            }
        }
    }
    m
}

/// Model one instruction, or `None` when it is not one this pass understands.
fn parse_insn(line: &str) -> Option<Inst> {
    // `call f #argregs N`, as the MIR code generator writes every call: the
    // call reads only its first N argument registers. Without the note a call
    // is taken to read all of them (and `%rax`, `%r10`, `%r11`), which kept a
    // dead copy into `%r8` or `%r9` alive before a one-argument call.
    let (t, argregs) = match line.trim().split_once(" #argregs ") {
        Some((code, n)) => (code.trim_end(), Some(n.trim().parse::<usize>().ok().filter(|n| *n <= 6)?)),
        None => (line.trim(), None),
    };
    let (mnem, rest) = match t.split_once(char::is_whitespace) {
        Some((m, r)) => (m, r.trim()),
        None => (t, ""),
    };
    let mut i = Inst::blank(line, Kind::Insn);
    i.mnem = mnem.to_string();
    // `rep movsq` and friends: the string instruction is not an operand.
    if mnem == "rep" {
        if !matches!(rest, "movsb" | "movsq" | "stosb" | "stosq") {
            return None;
        }
        i.ops = vec![Opnd::Other(rest.to_string())];
        i.read_only = vec![false];
        i.uses = bit(RCX) | bit(RSI) | bit(RDI) | bit(RAX);
        i.writes = bit(RCX) | bit(RSI) | bit(RDI);
        i.implicit = i.uses;
        i.reads_mem = true;
        i.writes_mem = true;
        return Some(i);
    }
    i.ops = split_operands(rest).into_iter().map(parse_opnd).collect();
    i.read_only = vec![false; i.ops.len()];
    // Every memory operand reads its address registers.
    for o in &i.ops {
        if let Opnd::Mem(m) = o {
            i.uses |= m.regs();
        }
    }
    let n = i.ops.len();
    let reg = |o: &Opnd| match o {
        Opnd::Reg(r, w) => Some((*r, *w)),
        _ => None,
    };
    let is_mem = |o: &Opnd| matches!(o, Opnd::Mem(_));
    let is_xmm = |o: &Opnd| matches!(o, Opnd::Other(s) if s.starts_with("%xmm"));
    let any_other = i.ops.iter().any(|o| matches!(o, Opnd::Other(_)));
    let b = base_mnem(mnem);

    // Jumps and calls first: their operand is a label or a symbol.
    if b == "jmp" {
        match i.ops.as_slice() {
            [Opnd::Other(l)] if !l.starts_with('*') => {
                i.flow = Flow::Jmp;
                i.target = Some(l.clone());
                return Some(i);
            }
            _ => return None,
        }
    }
    if argregs.is_some() && b != "call" {
        return None;
    }
    if b == "call" {
        let reads = match argregs {
            Some(n) => ARG_ORDER[..n].iter().fold(0, |set, r| set | bit(*r)),
            None => CALL_USES,
        };
        i.is_call = true;
        i.uses |= reads;
        i.writes = CALL_CLOBBERS;
        i.implicit = reads | CALL_CLOBBERS;
        i.reads_mem = true;
        i.writes_mem = true;
        match i.ops.as_slice() {
            [Opnd::Other(l)] => {
                // `call *%reg`: the register is read.
                if let Some(r) = l.strip_prefix('*') {
                    match parse_opnd(r) {
                        Opnd::Reg(r, 8) => i.uses |= bit(r),
                        Opnd::Mem(m) => i.uses |= m.regs(),
                        _ => return None,
                    }
                }
                return Some(i);
            }
            _ => return None,
        }
    }
    if b.starts_with('j') && b.len() <= 4 {
        match i.ops.as_slice() {
            [Opnd::Other(l)] if !l.starts_with('*') => {
                i.flow = Flow::Jcc;
                i.target = Some(l.clone());
                return Some(i);
            }
            _ => return None,
        }
    }

    // SSE: `%xmm` operands carry no general-purpose register; a
    // general-purpose operand is written by the conversions *to* integer and
    // by `movq`/`movd` out of an `%xmm`, and read everywhere else.
    if i.ops.iter().any(is_xmm) {
        const SSE: &[&str] = &[
            "movq", "movd", "movsd", "movss", "addsd", "addss", "subsd", "subss", "mulsd",
            "mulss", "divsd", "divss", "ucomisd", "ucomiss", "cvtss2sd", "cvtsd2ss", "cvtsi2sd",
            "cvtsi2ss", "cvtsi2sdq", "cvtsi2ssq", "cvttsd2si", "cvttss2si", "cvttsd2siq",
            "cvttss2siq", "sqrtsd", "sqrtss", "xorps", "xorpd", "andps", "andpd",
        ];
        if n != 2 || !SSE.contains(&mnem) {
            return None;
        }
        let to_int = matches!(
            mnem,
            "cvttsd2si" | "cvttss2si" | "cvttsd2siq" | "cvttss2siq" | "movq" | "movd"
        );
        let store = matches!(mnem, "movq" | "movd" | "movsd" | "movss");
        match &i.ops[0] {
            Opnd::Reg(r, _) => i.uses |= bit(*r),
            Opnd::Mem(_) => i.reads_mem = true,
            o if is_xmm(o) => {}
            _ => return None,
        }
        match &i.ops[1] {
            Opnd::Reg(r, w) => {
                if !to_int || *w < 4 {
                    return None;
                }
                i.writes = bit(*r);
                i.kills = bit(*r);
            }
            Opnd::Mem(_) => {
                if !store {
                    return None;
                }
                i.writes_mem = true;
            }
            o if is_xmm(o) => {}
            _ => return None,
        }
        return Some(i);
    }

    match b {
        "nop" if n == 0 => {}
        "ud2" | "hlt" if n == 0 => i.flow = Flow::Stop,
        "ret" if n == 0 => {
            i.flow = Flow::Stop;
            i.uses |= RET_USES;
            i.implicit = RET_USES;
            i.reads_mem = true;
        }
        "leave" if n == 0 => {
            // `mov %rbp, %rsp; pop %rbp`.
            i.uses |= bit(RBP);
            i.writes = bit(RSP) | bit(RBP);
            i.implicit = bit(RSP) | bit(RBP);
            i.reads_mem = true;
        }
        "syscall" if n == 0 => {
            i.uses |= bit(RAX) | bit(RDI) | bit(RSI) | bit(RDX) | bit(10) | bit(8) | bit(9);
            i.writes = bit(RAX) | bit(RCX) | bit(11);
            i.implicit = i.uses | i.writes;
            i.reads_mem = true;
            i.writes_mem = true;
        }
        "cqto" | "cqo" | "cltd" if n == 0 => {
            i.uses |= bit(RAX);
            i.kills = bit(RDX);
            i.writes = bit(RDX);
            i.implicit = bit(RAX) | bit(RDX);
        }
        "mov" | "movabs" if n == 2 && !any_other => {
            let byte = mnem == "movb";
            match (&i.ops[0], &i.ops[1]) {
                (src, Opnd::Reg(d, w)) => {
                    if let Some((s, _)) = reg(src) {
                        i.uses |= bit(s);
                        i.read_only[0] = true;
                    }
                    i.reads_mem = is_mem(src);
                    i.writes = bit(*d);
                    if *w >= 4 && !byte {
                        i.kills = bit(*d);
                        i.pure_def = Some(*d);
                        if *w == 8 {
                            i.kind = Kind::Mov;
                        }
                    } else {
                        // A byte or word write keeps the rest of the register.
                        i.uses |= bit(*d);
                    }
                }
                (src, Opnd::Mem(_)) => {
                    if let Some((s, _)) = reg(src) {
                        i.uses |= bit(s);
                        i.read_only[0] = true;
                    }
                    if is_mem(src) {
                        return None;
                    }
                    i.writes_mem = true;
                }
                _ => return None,
            }
        }
        "movzbl" | "movzbq" | "movzwl" | "movzwq" | "movsbl" | "movsbq" | "movswl" | "movswq"
        | "movslq" | "movzbw" | "movsbw"
            if n == 2 && !any_other =>
        {
            let (d, w) = reg(&i.ops[1])?;
            match &i.ops[0] {
                Opnd::Reg(s, _) => i.uses |= bit(*s),
                Opnd::Mem(_) => i.reads_mem = true,
                _ => return None,
            }
            i.writes = bit(d);
            if w >= 4 {
                i.kills = bit(d);
                i.pure_def = Some(d);
            } else {
                i.uses |= bit(d);
            }
        }
        "lea" if n == 2 => {
            let (d, w) = reg(&i.ops[1])?;
            if !is_mem(&i.ops[0]) || w < 4 {
                return None;
            }
            i.writes = bit(d);
            i.kills = bit(d);
            i.pure_def = Some(d);
            if w == 8 {
                i.kind = Kind::Lea;
            }
        }
        "add" | "sub" | "and" | "or" | "xor" | "adc" | "sbb" if n == 2 && !any_other => {
            match (&i.ops[0], &i.ops[1]) {
                (Opnd::Reg(s, sw), Opnd::Reg(d, dw)) if s == d && sw == dw && matches!(b, "xor" | "sub") && *dw >= 4 => {
                    // The zeroing idiom reads nothing.
                    i.writes = bit(*d);
                    i.kills = bit(*d);
                }
                (src, Opnd::Reg(d, _)) => {
                    if let Some((s, _)) = reg(src) {
                        i.uses |= bit(s);
                        i.read_only[0] = true;
                    }
                    i.reads_mem = is_mem(src);
                    i.uses |= bit(*d);
                    i.writes = bit(*d);
                }
                (src, Opnd::Mem(_)) => {
                    if is_mem(src) {
                        return None;
                    }
                    if let Some((s, _)) = reg(src) {
                        i.uses |= bit(s);
                        i.read_only[0] = true;
                    }
                    i.reads_mem = true;
                    i.writes_mem = true;
                }
                _ => return None,
            }
        }
        "cmp" | "test" if n == 2 && !any_other => {
            for k in 0..2 {
                match &i.ops[k] {
                    Opnd::Reg(r, _) => {
                        i.uses |= bit(*r);
                        i.read_only[k] = true;
                    }
                    Opnd::Mem(_) => i.reads_mem = true,
                    Opnd::Imm(_) => {}
                    Opnd::Other(_) => return None,
                }
            }
        }
        "imul" if n == 3 && !any_other => {
            let (d, _) = reg(&i.ops[2])?;
            match &i.ops[1] {
                Opnd::Reg(s, _) => {
                    i.uses |= bit(*s);
                    i.read_only[1] = true;
                }
                Opnd::Mem(_) => i.reads_mem = true,
                _ => return None,
            }
            i.writes = bit(d);
            i.kills = bit(d);
        }
        "imul" if n == 2 && !any_other => {
            let (d, _) = reg(&i.ops[1])?;
            match &i.ops[0] {
                Opnd::Reg(s, _) => {
                    i.uses |= bit(*s);
                    i.read_only[0] = true;
                }
                Opnd::Mem(_) => i.reads_mem = true,
                // `imul $k, %r` is `imul $k, %r, %r`.
                Opnd::Imm(_) => {}
                Opnd::Other(_) => return None,
            }
            i.uses |= bit(d);
            i.writes = bit(d);
        }
        "imul" | "mul" | "idiv" | "div" if n == 1 && !any_other => {
            match &i.ops[0] {
                Opnd::Reg(s, _) => {
                    i.uses |= bit(*s);
                    i.read_only[0] = true;
                }
                Opnd::Mem(_) => i.reads_mem = true,
                _ => return None,
            }
            i.uses |= bit(RAX);
            if matches!(b, "idiv" | "div") {
                i.uses |= bit(RDX);
            }
            i.writes = bit(RAX) | bit(RDX);
            i.implicit = bit(RAX) | bit(RDX);
        }
        "neg" | "not" | "inc" | "dec" | "bswap" if n == 1 && !any_other => match &i.ops[0] {
            Opnd::Reg(r, _) => {
                i.uses |= bit(*r);
                i.writes = bit(*r);
            }
            Opnd::Mem(_) => {
                i.reads_mem = true;
                i.writes_mem = true;
            }
            _ => return None,
        },
        "sal" | "shl" | "sar" | "shr" | "rol" | "ror" if (n == 1 || n == 2) && !any_other => {
            if n == 2 {
                match &i.ops[0] {
                    Opnd::Reg(RCX, 1) => {
                        i.uses |= bit(RCX);
                        i.implicit |= bit(RCX);
                    }
                    Opnd::Imm(_) => {}
                    _ => return None,
                }
            }
            match &i.ops[n - 1] {
                Opnd::Reg(r, _) => {
                    i.uses |= bit(*r);
                    i.writes = bit(*r);
                }
                Opnd::Mem(_) => {
                    i.reads_mem = true;
                    i.writes_mem = true;
                }
                _ => return None,
            }
        }
        "push" if n == 1 && !any_other => {
            match &i.ops[0] {
                Opnd::Reg(r, _) => i.uses |= bit(*r),
                Opnd::Mem(_) => i.reads_mem = true,
                Opnd::Imm(_) => {}
                Opnd::Other(_) => return None,
            }
            i.writes_mem = true;
        }
        "pop" if n == 1 => {
            let (r, _) = reg(&i.ops[0])?;
            i.writes = bit(r);
            i.kills = bit(r);
            i.reads_mem = true;
        }
        m if m.starts_with("set") && n == 1 => {
            // A byte write: the rest of the register survives.
            let (r, w) = reg(&i.ops[0])?;
            if w != 1 {
                return None;
            }
            i.uses |= bit(r);
            i.writes = bit(r);
        }
        m if m.starts_with("cmov") && n == 2 && !any_other => {
            let (d, _) = reg(&i.ops[1])?;
            match &i.ops[0] {
                Opnd::Reg(s, _) => {
                    i.uses |= bit(*s);
                    i.read_only[0] = true;
                }
                Opnd::Mem(_) => i.reads_mem = true,
                _ => return None,
            }
            i.uses |= bit(d);
            i.writes = bit(d);
        }
        _ => return None,
    }
    Some(i)
}

struct Func {
    insts: Vec<Inst>,
    /// Label name -> index of its line.
    labels: HashMap<String, usize>,
    /// Labels some instruction names (a jump target, or an address taken).
    referenced: HashSet<String>,
}

/// Parse a function, or report the first line that is not modelled.
fn parse_body(body: &str) -> Result<Func, String> {
    let mut insts = Vec::new();
    let mut labels = HashMap::new();
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') {
            return Err(t.to_string());
        }
        if let Some(name) = t.strip_suffix(':') {
            if name.contains(char::is_whitespace) {
                return Err(t.to_string());
            }
            labels.insert(name.to_string(), insts.len());
            insts.push(Inst::blank(line, Kind::Label));
            continue;
        }
        if t.starts_with('.') {
            // Alignment and symbol attributes only; anything else (data, a
            // section switch) is not code this pass should walk through.
            let known = [".p2align", ".balign", ".globl", ".type", ".size"];
            if !known.iter().any(|d| t.starts_with(d)) {
                return Err(t.to_string());
            }
            insts.push(Inst::blank(line, Kind::Directive));
            continue;
        }
        insts.push(parse_insn(line).ok_or_else(|| t.to_string())?);
    }
    let mut referenced = HashSet::new();
    for i in &insts {
        if let Some(t) = &i.target {
            referenced.insert(t.clone());
        }
        for o in &i.ops {
            let s = match o {
                Opnd::Other(s) | Opnd::Imm(s) => s.as_str(),
                Opnd::Mem(m) if m.rip => m.disp.as_str(),
                _ => continue,
            };
            // Every label in a body starts `.L`; do not walk the label table
            // for an operand that cannot name one.
            if !s.contains(".L") {
                continue;
            }
            for l in labels.keys() {
                if s.contains(l.as_str()) {
                    referenced.insert(l.clone());
                }
            }
        }
    }
    Ok(Func { insts, labels, referenced })
}

impl Func {
    /// Register liveness after each instruction, or `None` when a jump leaves
    /// the function for somewhere other than its epilogue.
    fn live_out(&self) -> Option<Vec<RegSet>> {
        let n = self.insts.len();
        // Successors: the next line, and/or a label.
        let mut jump_to: Vec<Option<usize>> = vec![None; n];
        let mut to_exit: Vec<bool> = vec![false; n];
        for (k, i) in self.insts.iter().enumerate() {
            if let Some(t) = &i.target {
                match self.labels.get(t) {
                    Some(&at) => jump_to[k] = Some(at),
                    // The only label outside the body a body may jump to is
                    // its own epilogue.
                    None if t.starts_with(".Lret_") => to_exit[k] = true,
                    None => return None,
                }
            }
        }
        let mut live_in: Vec<RegSet> = vec![0; n + 1];
        live_in[n] = EXIT_LIVE;
        let mut live_out: Vec<RegSet> = vec![0; n];
        loop {
            let mut changed = false;
            for k in (0..n).rev() {
                let i = &self.insts[k];
                let mut out = match i.flow {
                    Flow::Next | Flow::Jcc => live_in[k + 1],
                    Flow::Jmp | Flow::Stop => 0,
                };
                if let Some(at) = jump_to[k] {
                    out |= live_in[at];
                }
                if to_exit[k] {
                    out |= EXIT_LIVE;
                }
                out |= FIXED;
                let inn = if i.dead { out } else { (out & !i.kills) | i.uses };
                if out != live_out[k] || inn != live_in[k] {
                    live_out[k] = out;
                    live_in[k] = inn;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        Some(live_out)
    }

    /// Whether facts about register contents survive past line `k` into the
    /// next: not past an unconditional jump, and not into a label something
    /// jumps to.
    fn resets_before(&self, k: usize) -> bool {
        let i = &self.insts[k];
        if i.kind == Kind::Label {
            let name = i.text.trim().trim_end_matches(':');
            return self.referenced.contains(name);
        }
        false
    }

    /// Forward copy propagation and address forwarding, one extended basic
    /// block at a time.
    fn propagate(&mut self) -> Option<bool> {
        let mut changed = false;
        let mut copy_of: [Option<u8>; 16] = [None; 16];
        let mut lea_of: Vec<Option<Mem>> = vec![None; 16];
        let reset = |c: &mut [Option<u8>; 16], l: &mut Vec<Option<Mem>>| {
            *c = [None; 16];
            l.iter_mut().for_each(|x| *x = None);
        };
        for k in 0..self.insts.len() {
            if self.resets_before(k) {
                reset(&mut copy_of, &mut lea_of);
            }
            let i = &mut self.insts[k];
            if i.dead || matches!(i.kind, Kind::Label | Kind::Directive) {
                continue;
            }
            let mut touched = false;
            // Substitute copies in read-only register operands and in address
            // registers.
            for (o, ro) in i.ops.iter_mut().zip(i.read_only.iter()) {
                match o {
                    Opnd::Reg(r, 8) if *ro => {
                        if let Some(s) = copy_of[*r as usize] {
                            *r = s;
                            touched = true;
                        }
                    }
                    Opnd::Mem(m) => {
                        for slot in [&mut m.base, &mut m.index] {
                            if let Some(r) = slot {
                                if let Some(s) = copy_of[*r as usize] {
                                    *r = s;
                                    touched = true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // Forward a `lea` into a memory operand based on its result.
            for o in i.ops.iter_mut() {
                let Opnd::Mem(m) = o else { continue };
                let Some(b) = m.base else { continue };
                let Some(src) = &lea_of[b as usize] else { continue };
                if m.index.is_some() && src.index.is_some() {
                    continue;
                }
                let (Some(d0), Some(d1)) = (src.num_disp(), m.num_disp()) else { continue };
                let Some(d) = d0.checked_add(d1) else { continue };
                if d < i32::MIN as i64 / 2 || d > i32::MAX as i64 / 2 {
                    continue;
                }
                let (index, scale) = match (src.index, m.index) {
                    (Some(x), _) => (Some(x), src.scale),
                    (None, Some(x)) => (Some(x), m.scale),
                    (None, None) => (None, 1),
                };
                *m = Mem {
                    disp: if d == 0 && src.base.is_some() { String::new() } else { d.to_string() },
                    base: src.base,
                    index,
                    scale,
                    rip: false,
                };
                touched = true;
            }
            if touched {
                // Operands changed: the use set follows them.
                let line = {
                    i.render();
                    i.text.clone()
                };
                // Re-model the rewritten line. It cannot fail (only register
                // names and an address changed), but an instruction this pass
                // does not understand must not be reasoned about: give up on
                // the whole function.
                *i = parse_insn(&line)?;
                changed = true;
            }
            let i = &mut self.insts[k];
            // A move of a register onto itself does nothing.
            if i.kind == Kind::Mov {
                if let (Opnd::Reg(a, 8), Opnd::Reg(b, 8)) = (&i.ops[0], &i.ops[1]) {
                    if a == b {
                        i.dead = true;
                        changed = true;
                        continue;
                    }
                }
            }
            // Forget what this instruction overwrites.
            let w = i.writes;
            for r in 0..16u8 {
                if w & bit(r) != 0 {
                    copy_of[r as usize] = None;
                    lea_of[r as usize] = None;
                }
                if let Some(s) = copy_of[r as usize] {
                    if w & bit(s) != 0 {
                        copy_of[r as usize] = None;
                    }
                }
                if let Some(m) = &lea_of[r as usize] {
                    if m.regs() & w != 0 {
                        lea_of[r as usize] = None;
                    }
                }
            }
            // Record what it establishes.
            match (i.kind, i.ops.as_slice()) {
                (Kind::Mov, [Opnd::Reg(a, 8), Opnd::Reg(b, 8)]) if FIXED & (bit(*a) | bit(*b)) == 0 => {
                    copy_of[*b as usize] = Some(*a);
                }
                (Kind::Lea, [Opnd::Mem(m), Opnd::Reg(d, 8)])
                    if m.num_disp().is_some() && m.regs() & bit(*d) == 0 && FIXED & bit(*d) == 0 =>
                {
                    lea_of[*d as usize] = Some(m.clone());
                }
                _ => {}
            }
            if matches!(i.flow, Flow::Jmp | Flow::Stop) {
                reset(&mut copy_of, &mut lea_of);
            }
        }
        Some(changed)
    }

    /// `...compute into %t...; mov %t, %d` with `%t` dead afterwards: compute
    /// into `%d` and drop the move.
    fn retarget(&mut self, live_out: &[RegSet]) -> Option<bool> {
        for j in 0..self.insts.len() {
            let (t, d) = {
                let i = &self.insts[j];
                if i.dead || i.kind != Kind::Mov {
                    continue;
                }
                match i.ops.as_slice() {
                    [Opnd::Reg(t, 8), Opnd::Reg(d, 8)] if t != d => (*t, *d),
                    _ => continue,
                }
            };
            if FIXED & (bit(t) | bit(d)) != 0 || live_out[j] & bit(t) != 0 {
                continue;
            }
            // Walk back to the instruction that starts `%t`'s value.
            let mut start = None;
            let mut k = j;
            while k > 0 {
                k -= 1;
                let i = &self.insts[k];
                if i.dead || i.kind == Kind::Directive {
                    continue;
                }
                if i.kind == Kind::Label || i.flow != Flow::Next || i.is_call {
                    break;
                }
                // `%d` must be untouched for the whole chain.
                if (i.uses | i.writes | i.implicit) & bit(d) != 0 {
                    break;
                }
                let mentions = (i.uses | i.writes) & bit(t) != 0;
                if !mentions {
                    continue;
                }
                // Only spelled-out, full- or 32-bit-width operands can be
                // renamed.
                if i.implicit & bit(t) != 0 {
                    break;
                }
                let narrow = i.ops.iter().any(|o| matches!(o, Opnd::Reg(r, w) if *r == t && *w < 4));
                if narrow {
                    break;
                }
                if i.kills & bit(t) != 0 && i.uses & bit(t) == 0 {
                    start = Some(k);
                    break;
                }
            }
            let Some(start) = start else { continue };
            for k in start..j {
                let i = &mut self.insts[k];
                if i.dead || (i.uses | i.writes) & bit(t) == 0 {
                    continue;
                }
                for o in i.ops.iter_mut() {
                    match o {
                        Opnd::Reg(r, _) if *r == t => *r = d,
                        Opnd::Mem(m) => {
                            if m.base == Some(t) {
                                m.base = Some(d);
                            }
                            if m.index == Some(t) {
                                m.index = Some(d);
                            }
                        }
                        _ => {}
                    }
                }
                i.render();
                let line = i.text.clone();
                *i = parse_insn(&line)?;
            }
            self.insts[j].dead = true;
            // Liveness is stale past this point; let the caller recompute.
            return Some(true);
        }
        Some(false)
    }

    /// Drop every flag-neutral definition of a dead register.
    fn drop_dead(&mut self, live_out: &[RegSet]) -> bool {
        let mut changed = false;
        for (k, i) in self.insts.iter_mut().enumerate() {
            if i.dead {
                continue;
            }
            if let Some(r) = i.pure_def {
                if FIXED & bit(r) == 0 && live_out[k] & bit(r) == 0 {
                    i.dead = true;
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Optimise one function. Returns the text unchanged when any line is outside
/// what this pass models.
pub fn optimize(body: &str) -> String {
    optimize_or_explain(body).0
}

/// `optimize`, also returning the line that made the pass give up, if one did.
pub fn optimize_or_explain(body: &str) -> (String, Option<String>) {
    match parse_body(body) {
        Ok(f) => (run(f, body), None),
        Err(line) => (body.to_string(), Some(line)),
    }
}

fn run(mut f: Func, body: &str) -> String {
    let mut any = false;
    // Each round can expose more (a deleted move frees a register for the
    // next round's retargeting); the bound only guards against a cycle.
    for _ in 0..8 {
        let Some(mut changed) = f.propagate() else { return body.to_string() };
        // Retargeting renames registers, which invalidates liveness, so it
        // makes one change per liveness computation.
        let mut budget = 4 * f.insts.len() + 16;
        loop {
            let Some(live) = f.live_out() else { return body.to_string() };
            if budget == 0 {
                break;
            }
            match f.retarget(&live) {
                None => return body.to_string(),
                Some(false) => break,
                Some(true) => {}
            }
            budget -= 1;
            changed = true;
        }
        loop {
            let Some(live) = f.live_out() else { return body.to_string() };
            if !f.drop_dead(&live) {
                break;
            }
            changed = true;
        }
        if !changed {
            break;
        }
        any = true;
    }
    if !any {
        return body.to_string();
    }
    let mut out = String::with_capacity(body.len());
    for i in &f.insts {
        if !i.dead {
            out.push_str(&i.text);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_reads_only_the_argument_registers_it_names() {
        // `%r8` is not an argument of a one-argument call, so the copy into it
        // is dead; `%rdi` is, so its setup stays.
        let body = "  mov %rbx, %r8\n  mov %r12, %rdi\n  call f #argregs 1\n  mov %rax, %rbx\n";
        assert_eq!(optimize(body), "  mov %r12, %rdi\n  call f #argregs 1\n  mov %rax, %rbx\n");
        // Without the note every argument register counts as read.
        let plain = "  mov %rbx, %r8\n  mov %r12, %rdi\n  call f\n  mov %rax, %rbx\n";
        assert_eq!(optimize(plain), plain);
        // Five arguments: `%r8` is read.
        let five = "  mov %rbx, %r8\n  mov %r12, %rdi\n  call f #argregs 5\n  mov %rax, %rbx\n";
        assert_eq!(optimize(five), five);
    }

    #[test]
    fn an_argument_note_on_anything_but_a_call_is_not_modelled() {
        let body = "  mov %rbx, %r8 #argregs 1\n  ret\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_copy_is_read_through_and_then_dropped() {
        // `%r8` only carries `%rax` to the `xor`.
        let body = "  mov %rsi, %rax\n  sal $13, %rax\n  mov %rax, %r8\n  xor %r8, %rsi\n  mov %rsi, %rax\n";
        assert_eq!(optimize(body), "  mov %rsi, %rax\n  sal $13, %rax\n  xor %rax, %rsi\n  mov %rsi, %rax\n");
    }

    #[test]
    fn a_copy_that_is_still_needed_stays() {
        // `%r12` is read after `%rax` is overwritten.
        let body = "  mov %rax, %r12\n  mov $5, %rax\n  add %r12, %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_copy_is_not_read_through_once_its_source_changes() {
        // `%r12` holds the old `%rbx` and is read twice after `%rbx` moves on.
        let body = "  mov %rbx, %r12\n  add $1, %rbx\n  mov %r12, %rax\n  add %rbx, %rax\n  add %r12, %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_copy_read_once_after_its_source_changes_is_computed_in_place() {
        // Here the old value is only wanted in `%rax`, so it is put there
        // straight away instead of passing through `%r8`.
        let body = "  mov %rbx, %r8\n  add $1, %rbx\n  mov %r8, %rax\n  add %rbx, %rax\n";
        assert_eq!(optimize(body), "  mov %rbx, %rax\n  add $1, %rbx\n  add %rbx, %rax\n");
    }

    #[test]
    fn an_address_is_folded_into_the_access() {
        let body = "  lea -200(%rbp,%rsi,8), %rax\n  mov (%rax), %rax\n  mov %rax, %r8\n  mov %r8, %rax\n  add %r8, %rax\n";
        // The load reads through the address; `%r8` was only ever a copy of
        // `%rax`, so every read goes to `%rax` and the copies disappear.
        assert_eq!(optimize(body), "  mov -200(%rbp,%rsi,8), %rax\n  add %rax, %rax\n");
    }

    #[test]
    fn an_address_is_not_folded_after_its_index_changes() {
        let body = "  lea -200(%rbp,%rsi,8), %rcx\n  add $1, %rsi\n  mov (%rcx), %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_dead_load_and_a_dead_lea_are_dropped_but_a_store_is_not() {
        let body = "  mov -8(%rbp), %rcx\n  lea 8(%rbx), %rsi\n  mov %rbx, -16(%rbp)\n  mov $1, %rax\n";
        assert_eq!(optimize(body), "  mov %rbx, -16(%rbp)\n  mov $1, %rax\n");
    }

    #[test]
    fn nothing_that_sets_flags_is_dropped() {
        // `%rcx` is dead, but the `add` sets the flags the `jne` reads.
        let body = "  add $1, %rcx\n  jne .Lret_f\n  mov $1, %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_value_live_into_a_branch_target_is_kept() {
        let body = "  mov %rbx, %r12\n  cmp $0, %rdi\n  je .L1\n  mov $1, %rax\n  jmp .Lret_f\n.L1:\n  mov %r12, %rax\n";
        // `%r12` is read at `.L1`; the copy there cannot be read through
        // (facts do not cross a jump target), so both moves stay.
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_call_reads_its_argument_registers_and_ends_copies() {
        let body = "  mov %rbx, %rdi\n  call f\n  mov %rdi, %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn implicit_registers_are_not_renamed() {
        // `%rax` feeds the one-operand `imul`; the value cannot be computed
        // elsewhere.
        let body = "  mov %rdi, %rax\n  imul %rcx\n  mov %rax, %rbx\n  mov $0, %rax\n  add %rbx, %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_computation_is_retargeted_into_its_destination() {
        let body = "  mov %r9, %rax\n  imul $10, %rax\n  mov %rax, %rdi\n  mov $0, %rax\n";
        assert_eq!(optimize(body), "  mov %r9, %rdi\n  imul $10, %rdi\n  mov $0, %rax\n");
    }

    #[test]
    fn a_self_move_is_dropped_but_a_32_bit_one_is_not() {
        let body = "  mov %rdi, %rdi\n  mov %eax, %eax\n";
        assert_eq!(optimize(body), "  mov %eax, %eax\n");
    }

    #[test]
    fn a_callee_saved_register_is_dead_after_the_epilogue_pops_it() {
        // `%r12` is restored by the `pop`, so its last body value is dead and
        // the copy into it can go; `%rbx` is not saved here, so the caller's
        // value in it is live at the `ret` and its move must stay.
        let body = "f:\n  push %r12\n  mov %rdi, %rax\n  mov %rax, %r12\n  mov %rsi, %rbx\n  add %r12, %rax\n.Lret_f:\n  pop %r12\n  ret\n";
        assert_eq!(
            optimize(body),
            "f:\n  push %r12\n  mov %rdi, %rax\n  mov %rsi, %rbx\n  add %rdi, %rax\n.Lret_f:\n  pop %r12\n  ret\n"
        );
    }

    #[test]
    fn an_unknown_instruction_leaves_the_body_alone() {
        let body = "  mov %rax, %r12\n  frobnicate %r12\n  mov %r12, %rax\n";
        assert_eq!(optimize(body), body);
    }

    #[test]
    fn a_byte_write_keeps_the_rest_of_the_register() {
        // `sete` writes one byte of `%rax`; the `mov` before it is not dead.
        let body = "  mov $0, %rax\n  sete %al\n  mov %rax, %rbx\n  mov %rbx, %rax\n";
        let out = optimize(body);
        assert!(out.contains("mov $0, %rax") && out.contains("sete %al"), "{out}");
    }
}
