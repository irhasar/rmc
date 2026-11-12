//! Direct x86-64 (System V, AT&T syntax) code generator.
//!
//! Expression results live in `%rax`; the temporaries a binary operator needs
//! are spilled, not to the machine stack but to the reserved spill area: the
//! first few go to callee-saved registers (`spill_regs`), the rest to frame
//! slots just past the locals. `%rsp` therefore never moves mid-function and a
//! call needs no alignment pad. There is no general register allocator: locals
//! stay in memory, except that the hottest few scalars in each function are
//! promoted into callee-saved registers for the whole function (see `promote`).
//! A promoted local occupies a register instead of its stack slot; the rest
//! occupy an 8-byte slot at `-(slot + 1 + nregs)*8(%rbp)`, below the saved
//! registers and growing downward from `%rbp`.

use crate::ast::*;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

pub struct Codegen {
    out: String,
    label: u32,
    /// Current spill depth: number of values held in spill slots.
    sp_off: i32,
    /// First frame slot available for spills (just past the locals).
    spill_base: i32,
    /// Spill slots reserved for the function being generated, sized by a dry run.
    spill_slots: i32,
    /// High-water mark of `sp_off` during the current pass.
    max_spill: i32,
    str_id: u32,
    strings: Vec<(String, String)>, // (label, bytes)
    /// (continue_label, break_label, result_slot, label). `result_slot >= 0`
    /// for a value-yielding `loop`, whose `break value` stores into it before
    /// jumping to `break_label`; the loop loads it after `break_label`. `label`
    /// is the loop's own label, so a labelled `break`/`continue` finds its
    /// target (`None` for an unlabelled loop).
    loops: Vec<(String, String, i32, Option<String>)>,
    epilogue: String,
    /// Slot -> callee-saved register for locals promoted out of memory in the
    /// function currently being generated. Empty means every local is in memory.
    regs: Vec<(i32, &'static str)>,
    /// Callee-saved registers reserved for expression temporaries that must
    /// survive a call — the values `push_rax`/`pop_to` used to write to the
    /// frame. Holding the first few in a register turns each of those pairs into
    /// a register move, which is the whole win on call-heavy code such as `fib`.
    /// Trimmed to the number the function actually needs before it is emitted.
    spill_regs: Vec<&'static str>,
    /// When set, every array index is checked against the array's length and an
    /// out-of-range index traps instead of reading or writing adjacent memory.
    bounds: bool,
    /// When set, every `+`, `-` and `*` is followed by a jump-if-overflow to the
    /// runtime trap. Off by default because it costs a branch per operation.
    overflow: bool,
    /// True while generating an expression whose value is discarded (a
    /// statement-position expression). `if` without `else` then skips the
    /// `xor %eax, %eax` that materialises its unit value, which in `fib` is a
    /// dead instruction in the hot path.
    discard: bool,
    /// True during the throwaway dry run whose output is discarded. The dead
    /// and redundant instructions `trim_dead` removes do not affect the frame
    /// size or spill depth the dry run measures, so the pass is skipped there.
    dry: bool,
    /// Code that is correct but almost never runs, buffered here and appended
    /// after the function body so it does not sit in the hot fall-through path.
    /// The only producer is the 64-bit divide slow path; see `emit_div`.
    cold: String,
    /// True while `line`/`raw` write to `cold` instead of `out`.
    in_cold: bool,
    /// Frame slot holding the hidden return-buffer pointer for the function
    /// being generated (`-1` when the return type is scalar or unit).
    ret_slot: i32,
    /// Return type of the function being generated, needed to route `return`.
    ret_ty: Type,
    /// Number of frame slots (locals plus reserved spill slots) to allocate in
    /// the prologue. The parser sizes `nslots` for every local a function could
    /// declare; the code generator touches only the ones the emitted body
    /// actually names, so this is set to the deepest slot the body addresses.
    /// An all-register leaf like `fib` then needs no frame at all.
    ///
    /// Only meaningful during the final (emitting) pass; the dry pass leaves it
    /// at zero and the frame it emits is discarded. Recorded by `off` and
    /// `array_off`, so it is a `Cell`.
    frame_slots: Cell<i32>,
}

/// Callee-saved registers eligible for promoting a local out of memory. They
/// survive calls, so a promoted variable does not need to be spilled around
/// one. `%rbp` and `%rsp` are excluded; `%rbx` is listed first because a short
/// register name keeps the generated assembly readable.
const ALLOC_REGS: [&str; 5] = ["%rbx", "%r12", "%r13", "%r14", "%r15"];

/// How much a mention inside a loop body or condition is worth, relative to one
/// in straight-line code. The promotion heuristic ranks locals by how often they
/// are named, which alone ranks a cold temporary mentioned a few times on the
/// entry path above a scalar read once per iteration of the hot loop. Weighting
/// a loop mention this much keeps the loop-carried scalars in registers. It is a
/// fixed weight, not a trip-count estimate: the loop bounds are runtime values
/// here, and a scalar in any loop is a better register candidate than one in
/// none. The value only needs to dominate a plausible straight-line count, so a
/// small power of two is enough.
const LOOP_W: u32 = 32;

/// Callee-saved registers reserved for spill temporaries, after the locals have
/// taken what they need. The deepest is two at once (a binary op whose left
/// operand crosses a call in the right), so a handful covers every expression
/// the parser can build; past that the spill falls back to a frame slot.
const SPILL_REGS: [&str; 3] = ["%r12", "%r13", "%r14"];

/// A `let`-declared aggregate, recorded so promotion knows whether its
/// initialiser writes it field by field (`by_field`) or copies the whole value.
struct LetTarget {
    slot: i32,
    fields: i32,
    by_field: bool,
}

/// How a byte referent loads: `u8` zero-extends and `i8` sign-extends, so a
/// `*const u8` read of `0xFF` is `255` and a `*const i8` read is `-1`. A
/// non-byte referent returns `None` and uses a plain 64-bit load.
enum LoadKind {
    Zero,
    Sign,
}

fn byte_load(ty: &Type) -> Option<LoadKind> {
    match ty {
        Type::U8 => Some(LoadKind::Zero),
        Type::I8 => Some(LoadKind::Sign),
        _ => None,
    }
}

/// True when `target` is a byte-referent dereference, so its store must write
/// only the low byte (`movb`) rather than a word: a `*mut u8` write leaves the
/// other seven bytes of the slot alone.
fn is_byte_store(target: &Expr) -> bool {
    matches!(target, Expr::Deref { ty, .. } if matches!(ty, Type::U8 | Type::I8))
}

/// The value type a byte referent yields: `i64` (see `byte_load`). Mirrors
/// `sema::byte_value_ty`; the byte-ness is a property of the referent, not of
/// the value the program manipulates.
fn byte_value_ty(t: &Type) -> Type {
    match t {
        Type::U8 | Type::I8 => Type::I64,
        other => other.clone(),
    }
}

/// True for an integer width narrower than a machine word. A value of such a
/// type is kept truncated to its width in a 64-bit register, so a later use
/// (printing, division, a comparison) sees the same value rustc would.
fn is_narrow_int(t: &Type) -> bool {
    matches!(t, Type::Int { bits, .. } if *bits < 64)
}

/// The bit width of a narrow integer, or `None` for anything else.
fn narrow_bits(t: &Type) -> Option<u16> {
    match t {
        Type::Int { bits, .. } if *bits < 64 => Some(*bits),
        _ => None,
    }
}

/// True for a 64-bit general-purpose register name, so `mov %rax, %rax` can be
/// dropped as a no-op while `mov %eax, %eax` (a zero-extend) is kept.
fn is_64bit_reg(s: &str) -> bool {
    matches!(
        s,
        "%rax" | "%rbx" | "%rcx" | "%rdx" | "%rsi" | "%rdi" | "%rbp" | "%rsp"
            | "%r8" | "%r9" | "%r10" | "%r11" | "%r12" | "%r13" | "%r14" | "%r15"
    )
}

impl Codegen {
    pub fn new() -> Self {
        Codegen {
            out: String::with_capacity(1 << 16),
            label: 0,
            sp_off: 0,
            spill_base: 0,
            spill_slots: 0,
            max_spill: 0,
            str_id: 0,
            strings: Vec::new(),
            loops: Vec::new(),
            epilogue: String::new(),
            regs: Vec::new(),
            spill_regs: Vec::new(),
            bounds: false,
            overflow: false,
            discard: false,
            ret_slot: -1,
            ret_ty: Type::Unit,
            frame_slots: Cell::new(0),
            dry: false,
            cold: String::new(),
            in_cold: false,
        }
    }

    /// Turn on overflow checks for `+`, `-` and `*`.
    pub fn overflow_checks(mut self) -> Self {
        self.overflow = true;
        self
    }

    /// Turn on array bounds checks.
    pub fn bounds_checks(mut self) -> Self {
        self.bounds = true;
        self
    }

    /// The length of the array `base`, when it can be determined. A block or
    /// `if` base gets its length from its tail, since that expression's type is
    /// the value's type. This must cover every base `array_elem_operand` and the
    /// index paths handle, or the check is silently skipped.
    fn array_len_of(base: &Expr) -> Option<usize> {
        match base {
            Expr::Var { ty, .. } | Expr::Field { ty, .. } => Self::array_len_ty(ty),
            Expr::Call { ret_ty, .. } => Self::array_len_ty(ret_ty),
            // `m[i][j]`: the inner base `m[i]` has the element type of `m`.
            Expr::Index(b, _) => Self::array_elem_ty(b).and_then(|t| Self::array_len_ty(&t)),
            Expr::Block(b) => b.tail.as_deref().and_then(Self::array_len_of),
            Expr::MatchBoundary(b, _) => b.tail.as_deref().and_then(Self::array_len_of),
            // A loop expression's value is its `break` value, carried in the
            // loop's checked result type, not its block tail (which is `()` for
            // `loop { break a; }`). Missing this let `loop { break a; }[i]` skip
            // the bounds check on a variable index.
            Expr::Loop(_, _, lty, _) => Self::array_len_ty(lty),
            Expr::If(_, then, Some(els)) => Self::array_len_of_tail(then)
                .or_else(|| Self::array_len_of_tail(els)),
            _ => None,
        }
    }

    fn array_len_ty(ty: &Type) -> Option<usize> {
        match ty {
            Type::Array(_, n) => Some(*n),
            // Indexing an array reference checks against the referent's
            // length, so `&[T; N]` must report `N` or the check is skipped.
            Type::Ref(inner, _) => Self::array_len_ty(inner),
            _ => None,
        }
    }

    fn array_len_of_tail(b: &Block) -> Option<usize> {
        b.tail.as_deref().and_then(Self::array_len_of)
    }

    /// The element type of an array-valued expression `base`, when it can be
    /// determined. Mirrors `array_len_of` over the same base forms, so an
    /// element's stride is known wherever its length is. `None` means "assume a
    /// one-slot element", which is what a base of unknown type used to imply.
    fn array_elem_ty(base: &Expr) -> Option<Type> {
        let t = match base {
            Expr::Var { ty, .. } | Expr::Field { ty, .. } => ty,
            Expr::Call { ret_ty, .. } => ret_ty,
            Expr::Deref { ty, .. } => ty,
            // `m[i][j]`: the element of `m[i]` is the element of `m`'s element
            // type. Without this, an inner index of a nested array is taken to
            // have stride one and reads a neighbouring element.
            Expr::Index(b, _) => {
                return Self::array_elem_ty(b).and_then(|t| Self::array_elem_of_ty(&t))
            }
            Expr::Block(b) => return b.tail.as_deref().and_then(Self::array_elem_ty),
            Expr::MatchBoundary(b, _) => {
                return b.tail.as_deref().and_then(Self::array_elem_ty)
            }
            // Mirrors `array_len_of`: a loop expression's value type is its
            // checked result type, so an element's stride is known wherever its
            // length is.
            Expr::Loop(_, _, lty, _) => return Self::array_elem_of_ty(lty),
            Expr::If(_, then, Some(els)) => {
                return Self::array_elem_ty_tail(then)
                    .or_else(|| Self::array_elem_ty_tail(els))
            }
            _ => return None,
        };
        Self::array_elem_of_ty(t)
    }

    /// The element type of an array type, looking through a reference (which
    /// auto-derefs for indexing). `None` for a non-array.
    fn array_elem_of_ty(t: &Type) -> Option<Type> {
        let t = match t {
            Type::Ref(inner, _) => inner.as_ref(),
            other => other,
        };
        match t {
            Type::Array(e, _) => Some((**e).clone()),
            _ => None,
        }
    }

    fn array_elem_ty_tail(b: &Block) -> Option<Type> {
        b.tail.as_deref().and_then(Self::array_elem_ty)
    }

    /// The type of `e` when codegen can tell it statically. `gen_expr` already
    /// returns a type for every node, but a `&`/`|`/`^` on `bool` reports `i64`
    /// (the emitted instruction is the same for a 0/1 `bool` and an integer),
    /// so a consumer that must tell a bool from an integer — `!`, which is a
    /// logical not on `bool` and a bitwise not on `i64` — needs this. `None`
    /// means "not statically known".
    fn static_ty(e: &Expr) -> Option<Type> {
        match e {
            Expr::Bool(_) => Some(Type::Bool),
            Expr::Int(_) | Expr::Str(_) => Some(Type::I64),
            Expr::Char(_) => Some(Type::Char),
            Expr::Var { ty, .. } | Expr::Field { ty, .. } | Expr::Deref { ty, .. } => {
                Some(ty.clone())
            }
            Expr::Call { ret_ty, .. } => Some(ret_ty.clone()),
            Expr::Cast(_, to) => Some(to.clone()),
            Expr::Loop(_, _, lty, _) => Some(lty.clone()),
            Expr::Un(op, inner) => match op {
                UnOp::Neg => Self::static_ty(inner).filter(|t| is_narrow_int(t)).or(Some(Type::I64)),
                UnOp::Not => Self::static_ty(inner),
            },
            Expr::Bin(op, a, b) => match op {
                BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
                | BinOp::And | BinOp::Or => Some(Type::Bool),
                // The result of a bitwise op has the operand's type, so a
                // `bool` operand makes a `bool` result. Either side may carry
                // the type (`a[0] | a[1]`); a `bool` on either side wins because
                // sema rejects a mix of `bool` and `i64`.
                BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                    match (Self::static_ty(a), Self::static_ty(b)) {
                        (Some(Type::Bool), _) | (_, Some(Type::Bool)) => Some(Type::Bool),
                        (Some(Type::I64), _) | (_, Some(Type::I64)) => Some(Type::I64),
                        (Some(t), _) => Some(t),
                        (_, Some(t)) => Some(t),
                        _ => None,
                    }
                }
                // Arithmetic, shifts and `wrapping_mul` keep a narrow integer
                // width, so `gen_expr` truncates the 64-bit result to it. A
                // 64-bit or unknown operand is `i64`.
                _ => match (Self::static_ty(a), Self::static_ty(b)) {
                    (Some(t), _) if is_narrow_int(&t) => Some(t),
                    (_, Some(t)) if is_narrow_int(&t) => Some(t),
                    _ => Some(Type::I64),
                },
            },
            // An element of an array has the array's element type, which
            // `array_elem_ty` resolves (it looks through a reference too).
            Expr::Index(base, _) => Self::array_elem_ty(base),
            Expr::If(_, then, Some(els)) => {
                Self::static_ty_tail(then).or_else(|| Self::static_ty_tail(els))
            }
            Expr::Block(b) | Expr::MatchBoundary(b, _) => Self::static_ty_tail(b),
            _ => None,
        }
    }

    fn static_ty_tail(b: &Block) -> Option<Type> {
        b.tail.as_deref().and_then(Self::static_ty)
    }

    /// The stride of `base`'s elements in 8-byte slots. One for a scalar array,
    /// more for an array of nested arrays or structs.
    fn elem_stride(base: &Expr) -> usize {
        Self::array_elem_ty(base).map(|e| (e.size() / 8).max(1)).unwrap_or(1)
    }

    /// Emit a bounds check for `base[idx]`. For a variable index the value is
    /// in `%rax`, and `jae` after `cmp` catches both `idx >= len` and a negative
    /// index, which is an enormous unsigned value. A constant index is decided
    /// here instead: in range emits nothing, out of range is an unconditional
    /// trap.
    fn check_bounds(&mut self, base: &Expr, idx: &Expr) {
        if !self.bounds {
            return;
        }
        let n = match Self::array_len_of(base) {
            Some(n) => n,
            None => return,
        };
        if let Expr::Int(v) = idx {
            // A constant index is decided here: in range emits nothing, out of
            // range is a trap. (`v as i128` avoids wrapping on a huge literal.)
            if (*v as i128) < 0 || (*v as i128) >= n as i128 {
                self.line("jmp __rmc_bounds");
            }
            return;
        }
        if n <= i32::MAX as usize {
            self.line(&format!("cmp ${n}, %rax"));
            self.line("jae __rmc_bounds");
        }
    }

    /// The register holding local `slot`, if it was promoted out of memory.
    fn reg_of(&self, slot: i32) -> Option<&'static str> {
        self.regs.iter().find(|(s, _)| *s == slot).map(|(_, r)| *r)
    }

    fn line(&mut self, s: &str) {
        let buf = if self.in_cold { &mut self.cold } else { &mut self.out };
        buf.push_str("\t");
        buf.push_str(s);
        buf.push('\n');
    }

    fn raw(&mut self, s: &str) {
        let buf = if self.in_cold { &mut self.cold } else { &mut self.out };
        buf.push_str(s);
        buf.push('\n');
    }

    /// Route subsequent `line`/`raw` output into the cold buffer, to be
    /// appended after the function body. Paired with `cold_end`.
    fn cold_start(&mut self) {
        self.in_cold = true;
    }

    fn cold_end(&mut self) {
        self.in_cold = false;
    }

    /// Number of callee-saved registers currently holding promoted locals.
    fn nregs(&self) -> i32 {
        self.regs.len() as i32
    }

    /// Number of callee-saved registers saved below `%rbp`, holding promoted
    /// locals and spill temporaries. Both sit above the frame slots, so a slot
    /// offset is shifted past the sum.
    fn saved_regs(&self) -> i32 {
        self.nregs() + self.spill_regs.len() as i32
    }

    /// Byte offset of stack `slot` relative to `%rbp`. Slots grow downward, and
    /// the saved registers sit in the lowest bytes of the frame, so every slot
    /// is shifted past them and never overlaps `%rbp` or a saved register.
    fn off(&self, slot: i32) -> i32 {
        // Only a local slot sizes the frame; a spill slot lives in the reserved
        // region past the locals (`spill_slots`), so it must not be counted or it
        // would be added twice.
        if slot < self.spill_base {
            self.frame_slots.set(self.frame_slots.get().max(slot + 1));
        }
        -(slot + 1 + self.saved_regs()) * 8
    }

    /// Address of element 0 of an `n`-element array occupying `slot..slot+n`.
    /// Slots are laid out downward, so the lowest address is that of the last
    /// slot; elements are then addressed upward from there as `base + i*8`.
    fn array_off(&self, slot: i32, n: usize) -> i32 {
        if slot < self.spill_base {
            self.frame_slots.set(self.frame_slots.get().max(slot + n as i32));
        }
        -(slot + n as i32 + self.saved_regs()) * 8
    }

    fn new_label(&mut self) -> String {
        let l = format!(".L{}", self.label);
        self.label += 1;
        l
    }

    /// Spill `%rax` into a frame slot. A slot is used instead of `push` so that
    /// `%rsp` never moves mid-function: a moving stack forces a 16-byte
    /// alignment pad around every call, which costs more than the spill saves.
    fn push_rax(&mut self) {
        // The first few spills live in callee-saved registers, which survive
        // the calls the spill exists to cross; only the rest reach the frame.
        if let Some(r) = self.spill_regs.get(self.sp_off as usize) {
            let r = *r;
            self.line(&format!("mov %rax, {r}"));
        } else {
            // Frame spills are packed just past the locals, so a deeper
            // register-held spill does not leave a hole in the frame.
            let idx = self.spill_base + (self.sp_off - self.spill_regs.len() as i32);
            let o = self.off(idx);
            self.line(&format!("mov %rax, {o}(%rbp)"));
        }
        self.sp_off += 1;
        self.max_spill = self.max_spill.max(self.sp_off);
    }

    fn pop_to(&mut self, reg: &str) {
        self.sp_off -= 1;
        if let Some(r) = self.spill_regs.get(self.sp_off as usize) {
            let r = *r;
            self.line(&format!("mov {r}, {reg}"));
        } else {
            let idx = self.spill_base + (self.sp_off - self.spill_regs.len() as i32);
            let o = self.off(idx);
            self.line(&format!("mov {o}(%rbp), {reg}"));
        }
    }

    pub fn compile(mut self, prog: &Program) -> String {
        self.raw(".text");
        for f in &prog.fns {
            self.gen_fn(f);
        }
        // rodata: string literal records are `{len: i64, bytes...}`.
        if !self.strings.is_empty() {
            self.raw(".section .rodata");
            let strings = std::mem::take(&mut self.strings);
            for (label, bytes) in &strings {
                self.raw("\t.balign 8");
                self.raw(&format!("{label}:"));
                let b = bytes.as_bytes();
                self.raw(&format!("\t.quad {}", b.len()));
                if !b.is_empty() {
                    let mut hex = String::from("\t.byte ");
                    for (i, ch) in b.iter().enumerate() {
                        if i > 0 {
                            hex.push(',');
                        }
                        let _ = write!(hex, "{}", ch);
                    }
                    self.raw(&hex);
                }
            }
        }
        self.raw(".section .note.GNU-stack,\"\",@progbits");
        self.out
    }

    fn gen_fn(&mut self, f: &FnDef) {
        // Promote the most-used scalar locals into callee-saved registers.
        self.regs = Self::promote(f);
        // Spill slots live just past the locals in the frame. Generate the body
        // once to find the deepest spill, then regenerate with exactly that many
        // slots; the dry run's output is discarded.
        self.spill_base = f.nslots;
        self.spill_slots = 0;
        self.spill_regs = Vec::new();
        // Reset the frame size; `off`/`array_off` re-measure it as the dry pass
        // emits, and the final pass is seeded with the dry pass's result.
        self.frame_slots.set(0);
        let mark = (self.out.len(), self.label, self.str_id, self.strings.len());
        self.dry = true;
        // Dry run with every spill in the frame: it reports the deepest spill,
        // which is exactly how many spill registers are worth reserving, and the
        // deepest local slot the body addresses, which sizes the frame.
        self.gen_fn_body(f);
        let need = self.max_spill;
        let frame = self.frame_slots.get();
        self.out.truncate(mark.0);
        self.label = mark.1;
        self.str_id = mark.2;
        self.strings.truncate(mark.3);
        // Hold the first `nres` spills in callee-saved registers; the dry run
        // already proved nothing spills deeper than that. The pool is the same
        // registers the promoted locals draw on, so a spill register must be one
        // no local is already using — a promoted `%r12` and a spill `%r12` would
        // clobber each other.
        let used: Vec<&'static str> = self.regs.iter().map(|(_, r)| *r).collect();
        let free: Vec<&'static str> =
            ALLOC_REGS.iter().copied().filter(|r| !used.contains(r)).collect();
        let nres = (need as usize).min(SPILL_REGS.len()).min(free.len());
        self.spill_regs = free[..nres].to_vec();
        // Only spills past the reserved registers land in the frame; the rest
        // are held in `spill_regs`, so the frame needs just the remainder.
        self.spill_slots = (need as usize - nres) as i32;
        // Seed the frame size measured by the dry run so the prologue, which is
        // emitted before the body, allocates exactly that much, and start the
        // spill slots right after the deepest local actually used rather than
        // after every declared local; `gen_fn_body` then re-measures and must
        // agree (the extra spill slots do not change which locals are addressed).
        self.frame_slots.set(frame);
        self.spill_base = frame;
        self.dry = false;
        self.gen_fn_body(f);
        self.regs.clear();
        self.spill_regs.clear();
    }

    fn gen_fn_body(&mut self, f: &FnDef) {
        let fn_start = self.out.len();
        self.cold.clear();
        self.in_cold = false;
        self.sp_off = 0;
        self.max_spill = 0;
        self.epilogue = self.new_label();
        self.ret_slot = f.ret_slot;
        self.ret_ty = f.ret.clone();
        let mut saved: Vec<&'static str> = self.regs.iter().map(|(_, r)| *r).collect();
        // Spill registers are saved after the promoted locals, in pool order,
        // which is the order `off` accounts for them in.
        saved.extend(self.spill_regs.iter().copied());

        let nregs = saved.len() as i32;
        // The frame pointer is only needed when something is addressed through
        // it: a local slot, a spill slot, or an aggregate's frame buffer. When
        // the whole function lives in registers (`fib`), `%rbp` would cost a
        // push, a `mov` and a `leave` on every call for nothing, so it is left
        // out and `%rsp` alone carries the frame. Captured before the body is
        // emitted because emitting re-measures `frame_slots`.
        let uses_frame = self.frame_slots.get() > 0 || self.spill_slots > 0;
        self.raw(&format!(".globl {}", f.name));
        self.raw(&format!(".type {}, @function", f.name));
        self.raw(&format!("{}:", f.name));
        if uses_frame {
            self.line("push %rbp");
            self.line("mov %rsp, %rbp");
        }
        // Save the registers used for promoted locals.
        for r in &saved {
            self.line(&format!("push {r}"));
        }
        let saved_bytes = nregs as i32 * 8;
        let slots_bytes = (self.frame_slots.get() + self.spill_slots) * 8;
        // `%rsp` must be 16-byte aligned at every call: it is 8 mod 16 at entry
        // (the return address), so the total pushed must be 8 mod 16. With a
        // frame pointer `push %rbp` supplies the 8; without it the frame does.
        let frame = if uses_frame {
            ((slots_bytes + saved_bytes + 15) / 16) * 16 - saved_bytes
        } else {
            slots_bytes + (8 - saved_bytes - slots_bytes).rem_euclid(16)
        };
        if frame > 0 {
            self.line(&format!("sub ${frame}, %rsp"));
        }
        // An aggregate return is written through a pointer the caller leaves in
        // `%r10`; stash it in its slot before anything can clobber it.
        if f.ret.is_aggregate() {
            self.line(&format!("mov %r10, {}(%rbp)", self.off(f.ret_slot)));
        }
        // Spill incoming arguments. A scalar goes into its slot or promoted
        // register. An aggregate arrives as a pointer and is copied into the
        // parameter's frame buffer so the callee owns a private copy.
        //
        // The copy clobbers `%rsi`/`%rdi`, which are also where later arguments
        // live, so the aggregate pointers are saved on the stack first and the
        // copies run afterwards. Scalars are spilled up front too; that touches
        // neither of those registers.
        const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
        for (i, (_, pty)) in f.params.iter().enumerate() {
            if i >= 6 {
                break;
            }
            let slot = f.param_slots[i];
            if pty.is_aggregate() {
                self.line(&format!("push {}", ARG_REGS[i]));
            } else {
                match self.reg_of(slot) {
                    Some(r) => self.line(&format!("mov {}, {r}", ARG_REGS[i])),
                    None => self.line(&format!("mov {}, {}(%rbp)", ARG_REGS[i], self.off(slot))),
                }
            }
        }
        // Pop the saved pointers in reverse and copy each argument into place.
        for (i, (_, pty)) in f.params.iter().enumerate().rev() {
            if i >= 6 {
                continue;
            }
            if pty.is_aggregate() {
                let n = pty.size() / 8;
                let slot = f.param_slots[i];
                self.line("pop %rsi");
                self.line(&format!("lea {}(%rbp), %rdi", self.array_off(slot, n)));
                self.copy_words_rdi_rsi(n);
            }
        }
        let ty = self.gen_block(&f.body);
        if f.ret.is_aggregate() {
            // Implicit return of an aggregate tail expression.
            if ty.is_aggregate() {
                let n = f.ret.size() / 8;
                self.gen_return_aggregate_from_rax(n);
            }
        } else if ty == Type::Unit {
            self.line("xor %eax, %eax");
        }
        self.raw(&format!("{}:", self.epilogue));
        if uses_frame {
            // The saved registers sit just below %rbp; point %rsp at the lowest
            // one and pop them back, which also discards the frame.
            if nregs > 0 {
                self.line(&format!("lea -{}(%rbp), %rsp", nregs * 8));
                for r in saved.iter().rev() {
                    self.line(&format!("pop {r}"));
                }
            }
            self.line("leave");
        } else {
            // No frame pointer: undo the frame allocation, then pop the saved
            // registers back to their caller's values.
            if frame > 0 {
                self.line(&format!("add ${frame}, %rsp"));
            }
            for r in saved.iter().rev() {
                self.line(&format!("pop {r}"));
            }
        }
        self.line("ret");
        // Append the cold blocks now that the hot path is complete.
        if !self.cold.is_empty() {
            let cold = std::mem::take(&mut self.cold);
            self.out.push_str(&cold);
        }
        self.raw(&format!(".size {}, .-{}", f.name, f.name));
        if !self.dry {
            self.trim_dead(fn_start);
        }
    }

    /// Drop instructions that cannot be reached because the instruction before
    /// them unconditionally transfers control (`jmp`, `ret`), drop a
    /// register-to-self `mov`, and drop a `jmp` whose target is the very next
    /// label (the fallthrough already goes there). The generator emits all
    /// three: a `return` inside an expression leaves a `jmp` to the epilogue
    /// followed by the rest of the expression, a value already in a destination
    /// register is still "stored", and a function whose last expression is a
    /// value jumps to an epilogue label that immediately follows. None changes
    /// behaviour but each costs I-cache and decode slots.
    ///
    /// A label ends the unreachable run: a later `jmp` may target it, so the
    /// instruction after a label is live even when the text before it is not.
    /// Output from `start` onward is rewritten in place; the pass is a fixpoint
    /// in one sweep because removing an instruction never creates a new label
    /// or a new terminator.
    fn trim_dead(&mut self, start: usize) {
        let mut head = self.out[..start].to_string();
        let body = self.out[start..].to_string();
        let mut dead = false;
        // The last live instruction, as (src, dst) for a `mov` and `None` for
        // anything else. Used to fold a store immediately undone by its reload
        // (or the reverse) into nothing.
        let mut prev: Option<(String, String)> = None;
        for raw_line in body.split_inclusive('\n') {
            let line = raw_line.trim_end_matches('\n');
            let is_label = !line.starts_with('\t') && !line.starts_with(' ') && line.ends_with(':');
            let text = line.trim_start();
            // A directive (`.p2align`, `.section`, ...) is live even when it
            // follows a `jmp`/`ret`, so it resets the unreachable run like a
            // label. It may be indented (`\t.p2align 4`), so test the trimmed
            // text; no instruction starts with `.`, so this cannot catch one.
            if is_label || text.starts_with('.') {
                dead = false;
                prev = None;
                if is_label {
                    // `jmp L` immediately followed by `L:` is a jump to the next
                    // instruction; drop it. The label is written with no leading
                    // tab, the jump with one, so the exact suffix identifies it.
                    let jmp_line = format!("\tjmp {}\n", line.trim_end_matches(':'));
                    if head.ends_with(&jmp_line) {
                        head.truncate(head.len() - jmp_line.len());
                    }
                }
            }
            if dead {
                continue;
            }
            if let Some(rest) = text.strip_prefix("mov ") {
                if let Some((src, dst)) = rest.split_once(',') {
                    let (src, dst) = (src.trim(), dst.trim());
                    // `mov %r, %r` with the same operand twice is a no-op — but
                    // only for a 64-bit register: `mov %eax, %eax` is a real
                    // instruction that zero-extends into `%rax`.
                    if src == dst && is_64bit_reg(src) {
                        continue;
                    }
                    // A `mov` whose source and destination are the reverse of
                    // the previous `mov`'s is a round trip that changes nothing
                    // (both are adjacent, so nothing clobbers the register in
                    // between): `mov %rax, X` then `mov X, %rax`. Drop the
                    // second and keep the first, so a store survives and a
                    // pointless reload disappears.
                    if let Some((psrc, pdst)) = &prev {
                        if src == pdst && dst == psrc {
                            continue;
                        }
                    }
                    prev = Some((src.to_string(), dst.to_string()));
                } else {
                    prev = None;
                }
            } else {
                prev = None;
            }
            if text == "jmp" || text.starts_with("jmp ") || text == "ret" {
                dead = true;
            }
            head.push_str(raw_line);
        }
        self.out = head;
    }

    /// Copy `n` 8-byte words from the address in `%rsi` to the address in `%rdi`.
    /// `%rax` and, for larger counts, `%rcx` are clobbered.
    fn copy_words_rdi_rsi(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        // Scratch is %r8/%r9, never %rax: a caller may hold a live value in
        // %rax across the copy (a pending operand, or an argument already
        // loaded into a register).
        if n <= 8 {
            for i in 0..n {
                self.line(&format!("mov {}(%rsi), %r8", i as i32 * 8));
                self.line(&format!("mov %r8, {}(%rdi)", i as i32 * 8));
            }
        } else {
            let l = self.new_label();
            self.line(&format!("mov ${n}, %rcx"));
            self.raw(&format!("{l}:"));
            self.line("mov (%rsi), %r8");
            self.line("mov %r8, (%rdi)");
            self.line("add $8, %rsi");
            self.line("add $8, %rdi");
            self.line("dec %rcx");
            self.line(&format!("jnz {l}"));
        }
    }

    /// Copy the aggregate whose address is in `%rax` into the hidden return
    /// buffer, then leave the buffer address in `%rax`.
    fn gen_return_aggregate_from_rax(&mut self, n: usize) {
        self.line("mov %rax, %rsi");
        self.line(&format!("mov {}(%rbp), %rdi", self.off(self.ret_slot)));
        self.copy_words_rdi_rsi(n);
        self.line(&format!("mov {}(%rbp), %rax", self.off(self.ret_slot)));
    }

    /// Choose up to `ALLOC_REGS.len()` slots to keep in registers for the whole
    /// function, ordered by how often they are mentioned.
    ///
    /// Scalars are counted directly. A struct's scalar fields are counted too,
    /// so a hot `acc.sum` stays in a register, but only when the struct is
    /// never read or written as a whole: a whole-struct copy runs through
    /// memory, so every field of such a struct must stay in memory. A field is
    /// named by its register only where the layout is known, so the struct's
    /// base slot (its last field, whose address is the struct's own base) and
    /// any aggregate field always stay in memory.
    fn promote(f: &FnDef) -> Vec<(i32, &'static str)> {
        // Sparse maps: a local that is never named never gets a key, so a
        // function with one large array does not pay for a dense table over
        // every element slot (which is what made codegen O(array length)).
        let mut counts: HashMap<i32, u32> = HashMap::new();
        let mut mem: HashSet<i32> = HashSet::new();
        // Structs used as a whole value, recorded as (base slot, field count).
        let mut copied: Vec<(i32, i32)> = Vec::new();
        // `let`-declared aggregates, which are copied as a whole unless their
        // initialiser writes them field by field.
        let mut let_tmps: Vec<LetTarget> = Vec::new();
        Self::count_block(&f.body, &mut counts, &mut mem, &mut copied, true, &mut let_tmps);

        // An aggregate parameter is copied into its slots in the prologue, so
        // every one of its fields must stay in memory for that copy to count.
        for (i, (_, pty)) in f.params.iter().enumerate() {
            if pty.is_aggregate() {
                let base = f.param_slots[i];
                for k in 0..(pty.size() / 8) as i32 {
                    mem.insert(base + k);
                }
            }
        }

        // A struct that is ever copied, returned or passed by value is read and
        // written through memory, so none of its fields may be promoted.
        for (base, fields) in &copied {
            for k in 0..*fields {
                mem.insert(base + k);
            }
        }

        // A `let` whose initialiser is not field-by-field copies the whole
        // value, so its fields must stay in memory too.
        for t in &let_tmps {
            if !t.by_field {
                for k in 0..t.fields {
                    mem.insert(t.slot + k);
                }
            }
        }

        let mut ranked: Vec<(i32, u32)> = counts
            .iter()
            .map(|(&s, &c)| (s, if mem.contains(&s) { 0 } else { c }))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked
            .into_iter()
            .filter(|(_, c)| *c > 0)
            .take(ALLOC_REGS.len())
            .zip(ALLOC_REGS.iter())
            .map(|((slot, _), reg)| (slot, *reg))
            .collect()
    }


    /// Fold a loop's own counts into the running total at `LOOP_W`, saturating so
    /// a deeply nested loop cannot overflow the counter and wrap a hot local to
    /// the bottom of the ranking.
    fn merge_loop(counts: &mut HashMap<i32, u32>, sub: HashMap<i32, u32>) {
        for (slot, c) in sub {
            let e = counts.entry(slot).or_insert(0);
            *e = e.saturating_add(c.saturating_mul(LOOP_W));
        }
    }

    fn count_block(
        b: &Block,
        counts: &mut HashMap<i32, u32>,
        mem: &mut HashSet<i32>,
        copied: &mut Vec<(i32, i32)>,
        whole: bool,
        let_tmps: &mut Vec<LetTarget>,
    ) {
        for s in &b.stmts {
            Self::count_stmt(s, counts, mem, copied, whole, let_tmps);
        }
        if let Some(t) = &b.tail {
            Self::count_expr(t, counts, mem, copied, whole, let_tmps);
        }
    }

    fn count_stmt(
        s: &Stmt,
        counts: &mut HashMap<i32, u32>,
        mem: &mut HashSet<i32>,
        copied: &mut Vec<(i32, i32)>,
        _whole: bool,
        let_tmps: &mut Vec<LetTarget>,
    ) {
        match s {
            Stmt::Let { ty, init, slot, .. } => {
                if let Some(e) = init {
                    Self::count_expr(e, counts, mem, copied, true, let_tmps);
                }
                // A `let` of an aggregate writes the whole value, except when
                // the initialiser is field-by-field: a struct literal, a splat
                // (`[v; n]`), or a zero-fill. Only then may its fields live in
                // registers.
                match ty {
                    Some(Type::Struct(_, size)) => {
                        let by_field = matches!(
                            init,
                            None | Some(Expr::StructLit { .. }) | Some(Expr::ArrayLit(..))
                        );
                        let_tmps.push(LetTarget {
                            slot: *slot,
                            fields: (*size / 8) as i32,
                            by_field,
                        });
                    }
                    Some(Type::Array(..)) => {
                        // Like any array, only the base slot can be named, so
                        // only it must stay in memory.
                        mem.insert(*slot);
                    }
                    // A payload enum is a tagged union copied word by word, so
                    // its words stay in memory like a whole struct.
                    Some(Type::EnumData(_, size)) => {
                        let_tmps.push(LetTarget {
                            slot: *slot,
                            fields: (*size / 8) as i32,
                            by_field: false,
                        });
                    }
                    _ => {}
                }
            }
            Stmt::Assign { target, value } => {
                // A whole-struct target is written as a copy, so its fields
                // must stay in memory just like a whole-struct source.
                Self::count_expr(target, counts, mem, copied, true, let_tmps);
                Self::count_expr(value, counts, mem, copied, true, let_tmps);
            }
            Stmt::Expr(e) => Self::count_expr(e, counts, mem, copied, true, let_tmps),
            Stmt::While(c, b, _) => {
                // A loop's condition and body run once per iteration, so a
                // mention there is worth `LOOP_W` mentions in straight-line
                // code: a scalar read only inside the loop should outrank a
                // colder one that is merely mentioned more times outside it.
                // The body is counted into a fresh map so the whole subtree is
                // weighted once; a nested loop inside it weights again.
                let mut sub = HashMap::new();
                Self::count_expr(c, &mut sub, mem, copied, true, let_tmps);
                Self::count_block(b, &mut sub, mem, copied, true, let_tmps);
                Self::merge_loop(counts, sub);
            }
            Stmt::For { start, end, body, var_slot, end_slot, .. } => {
                Self::count_expr(start, counts, mem, copied, true, let_tmps);
                Self::count_expr(end, counts, mem, copied, true, let_tmps);
                // The counter and the bound live in hidden slots no expression
                // names, yet both are read (and the counter written) every
                // iteration. Count them so they compete for a register like any
                // other loop-carried local instead of being reloaded from the
                // frame on each pass. They are counted at loop weight because
                // that is where they are touched.
                let mut sub = HashMap::new();
                *sub.entry(*var_slot).or_insert(0) += 1;
                if Self::small_imm(end).is_none() {
                    *sub.entry(*end_slot).or_insert(0) += 1;
                }
                Self::count_block(body, &mut sub, mem, copied, true, let_tmps);
                Self::merge_loop(counts, sub);
            }
            Stmt::Return(v) => {
                if let Some(e) = v {
                    Self::count_expr(e, counts, mem, copied, true, let_tmps);
                }
            }
            Stmt::Break(v, _) => {
                if let Some(e) = v {
                    Self::count_expr(e, counts, mem, copied, true, let_tmps);
                }
            }
            Stmt::Continue(_) => {}
        }
    }

    /// Marks every local that a `&` reaches as needing memory, so its frame
    /// slot stays current and `lea` yields a valid address.
    fn mark_addressed(e: &Expr, mem: &mut HashSet<i32>) {
        match e {
            Expr::Var { slot, ty, .. } => {
                // An array's address is its base slot's; its elements are
                // reached relative to that base and are never promoted.
                let n = if matches!(ty, Type::Array(..)) {
                    1
                } else if ty.is_aggregate() {
                    (ty.size() / 8).max(1)
                } else {
                    1
                };
                for k in 0..n as i32 {
                    mem.insert(*slot + k);
                }
            }
            Expr::Field { base, .. } => Self::mark_addressed(base, mem),
            Expr::Index(base, _) => Self::mark_addressed(base, mem),
            // A dereference's target already lives in memory.
            Expr::Deref { .. } => {}
            _ => {}
        }
    }

    /// `whole` is true when the value of `e` is used as a complete aggregate,
    /// which is what makes a struct unfit for per-field promotion. A `Field` or
    /// `Index` base is not a whole use, so it is passed `false`.
    fn count_expr(
        e: &Expr,
        counts: &mut HashMap<i32, u32>,
        mem: &mut HashSet<i32>,
        copied: &mut Vec<(i32, i32)>,
        whole: bool,
        let_tmps: &mut Vec<LetTarget>,
    ) {
        match e {
            Expr::ArrayList(_) => unreachable!("sema rewrites ArrayList into a block"),
            Expr::TupleLit(_) => unreachable!("sema rewrites TupleLit into a StructLit"),
            Expr::MethodCall { .. } => unreachable!("sema rewrites MethodCall into a Call"),
Expr::PatScrutinee(_) => {
                unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
            }
            Expr::PatCheck { .. } => unreachable!("sema rewrites PatCheck into `true`"),
            Expr::PatArity { .. } => unreachable!("sema rewrites PatArity into `true`"),
            Expr::EnumVariant { .. } => {} // a tag has no subexpressions
            Expr::EnumCtor { args, slot, size, .. } => {
                for a in args {
                    Self::count_expr(a, counts, mem, copied, true, let_tmps);
                }
                // The ctor yields its own address, so its words stay in memory.
                for k in 0..(*size / 8) as i32 {
                    mem.insert(*slot + k);
                }
            }
            // A tag test reads the value's first word, and a payload read
            // offsets into it: both need the value's address, never a copy.
            Expr::VariantTest { scrutinee, .. } => {
                Self::count_expr(scrutinee, counts, mem, copied, false, let_tmps)
            }
            Expr::BoolTest { scrutinee, .. } => {
                Self::count_expr(scrutinee, counts, mem, copied, false, let_tmps)
            }
            Expr::EnumPayload { recv, ty, .. } => {
                Self::count_expr(recv, counts, mem, copied, false, let_tmps);
                // A payload field of aggregate type is named by its address.
                let _ = ty;
            }
            Expr::Var { slot, ty, .. } => {
                if !ty.is_aggregate() {
                    *counts.entry(*slot).or_insert(0) += 1;
                }
                if ty.is_aggregate() {
                    match ty {
                        Type::Array(..) => {
                            // An array is only ever reached through its base
                            // address, so its base slot must stay in memory.
                            // Element slots are never counted (nothing names
                            // them) and so are never promoted; marking them
                            // here would be O(length) per mention for no gain.
                            mem.insert(*slot);
                        }
                        // A struct's base slot is its *last* field. It is only
                        // needed when the value is copied as a whole, which is
                        // what `copied` records; a field-by-field use reaches
                        // every field by address off `array_off`, so the base
                        // slot stays promotable and the last field competes for
                        // a register like any other.
                        Type::Struct(_, size) => {
                            if whole {
                                copied.push((*slot, (*size / 8) as i32));
                            }
                        }
                        // A payload enum is a tagged union read as a whole, so
                        // its base word stays in memory.
                        Type::EnumData(_, size) => {
                            mem.insert(*slot);
                            if whole {
                                copied.push((*slot, (*size / 8) as i32));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Expr::Index(base, idx) => {
                Self::count_expr(base, counts, mem, copied, false, let_tmps);
                Self::count_expr(idx, counts, mem, copied, false, let_tmps);
            }
            Expr::Field { base, offset, ty, .. } => {
                Self::count_expr(base, counts, mem, copied, false, let_tmps);
                if let Expr::Var { slot, ty: Type::Struct(_, size), .. } = base.as_ref() {
                    let fields = (*size / 8) as i32;
                    let fslot = *slot + fields - 1 - *offset / 8;
                    if fslot >= 0 {
                        if ty.is_aggregate() {
                            for k in 0..(ty.size() / 8) as i32 {
                                mem.insert(fslot - k);
                            }
                        } else {
                            *counts.entry(fslot).or_insert(0) += 1;
                        }
                    }
                }
            }
            Expr::StructLit { fields, slot, .. } => {
                for fi in fields {
                    Self::count_expr(&fi.value, counts, mem, copied, true, let_tmps);
                }
                // The literal yields its own address, so it is a whole use of
                // itself: its fields stay in memory for that copy. Sema gives a
                // literal an inferred type only as a `let` initialiser, which is
                // already marked whole by that `let`.
                let n = fields.len() as i32;
                for k in 0..n {
                    mem.insert(*slot + k);
                }
            }
            Expr::Bin(_, a, b) => {
                Self::count_expr(a, counts, mem, copied, true, let_tmps);
                Self::count_expr(b, counts, mem, copied, true, let_tmps);
            }
            Expr::Un(_, a) | Expr::Cast(a, _) => {
                Self::count_expr(a, counts, mem, copied, true, let_tmps)
            }
            // `&e` on a place is that place's address, so `e` is read as a
            // place, never as a whole value to copy. A temporary `&e` (slot >=
            // 0) keeps the temporary's words in memory for the copy.
            Expr::AddrOf { expr, ty, slot, .. } => {
                // Taking a place's address needs the place to be in memory: a
                // register-promoted local has no address, and `lea slot(%rbp)`
                // would read a stale frame word.
                Self::mark_addressed(expr, mem);
                Self::count_expr(expr, counts, mem, copied, false, let_tmps);
                if *slot >= 0 {
                    for k in 0..(ty.size() / 8).max(1) as i32 {
                        mem.insert(*slot + k);
                    }
                }
            }
            Expr::Deref { expr, .. } => {
                Self::count_expr(expr, counts, mem, copied, false, let_tmps)
            }
            Expr::Call { args, .. } => {
                for a in args {
                    Self::count_expr(a, counts, mem, copied, true, let_tmps);
                }
            }
            Expr::If(c, t, els) => {
                Self::count_expr(c, counts, mem, copied, true, let_tmps);
                Self::count_block(t, counts, mem, copied, true, let_tmps);
                if let Some(e) = els {
                    Self::count_block(e, counts, mem, copied, true, let_tmps);
                }
            }
            Expr::Block(b) => Self::count_block(b, counts, mem, copied, true, let_tmps),
            Expr::MatchBoundary(b, _) | Expr::Loop(b, _, _, _) => {
                Self::count_block(b, counts, mem, copied, true, let_tmps)
            }
            Expr::ArrayLit(v, _) => {
                Self::count_expr(v, counts, mem, copied, true, let_tmps)
            }
            Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) => {}
        }
    }

    /// Generate an expression whose value is discarded, so `if` without `else`
    /// need not materialise its unit result. Only a bare `if` is special-cased:
    /// a block or `match` in statement position is left alone, because their
    /// `if` arms can appear in value position within the block's own tail.
    fn gen_stmt_expr(&mut self, e: &Expr) {
        let saved = self.discard;
        self.discard = true;
        self.gen_expr(e);
        self.discard = saved;
    }

    /// The label to branch to when an `if`-without-`else` body is a lone
    /// `break`/`continue`, so the branch can target the loop's exit/continue
    /// label directly and skip the `jmp`. Returns `None` when the body is
    /// anything else (including a `break` nested in a block), which falls back
    /// to the ordinary `if` lowering.
    fn bare_exit(&self, then: &Block, els: &Option<Block>) -> Option<String> {
        if els.is_some() || then.tail.is_some() || then.stmts.len() != 1 {
            return None;
        }
        let (top, end, _, _) = self.loops.last()?;
        match &then.stmts[0] {
            // Only an unlabelled `break`/`continue` targets the innermost loop;
            // a labelled one may name an outer loop, so it falls back to the
            // ordinary `if` lowering.
            Stmt::Break(None, None) => Some(end.clone()),
            Stmt::Continue(None) => Some(top.clone()),
            _ => None,
        }
    }

    /// The loop a `break`/`continue` targets, as an index into `loops`: the
    /// innermost loop when `label` is `None`, else the innermost loop carrying
    /// that label. Sema has already rejected an unmatched label, so the lookup
    /// always succeeds here.
    fn loop_target(&self, label: &Option<String>) -> usize {
        match label {
            None => self.loops.len() - 1,
            Some(name) => self
                .loops
                .iter()
                .rposition(|l| l.3.as_deref() == Some(name))
                .expect("sema checked the label"),
        }
    }

    /// Generate a `loop` in value position. The body repeats until a `break`
    /// jumps to `end`. When `slot >= 0` the loop yields a value: a `break value`
    /// stores into the slot before jumping, and `end` is followed by a load of
    /// that slot so the value is in `%rax`. A unit/`!` loop (`slot < 0`) needs
    /// neither.
    fn gen_loop_expr(&mut self, body: &Block, slot: i32, label: &Option<String>) {
        let top = self.new_label();
        let end = self.new_label();
        self.loops.push((top.clone(), end.clone(), slot, label.clone()));
        self.raw("\t.p2align 4");
        self.raw(&format!("{top}:"));
        self.gen_block(body);
        self.line(&format!("jmp {top}"));
        self.raw(&format!("{end}:"));
        self.loops.pop();
        if slot >= 0 {
            self.gen_var_operand(slot, "%rax");
        }
    }

    fn gen_block(&mut self, b: &Block) -> Type {
        for st in &b.stmts {
            self.gen_stmt(st);
        }
        match &b.tail {
            Some(e) => self.gen_expr(e),
            None => Type::Unit,
        }
    }

    fn gen_stmt(&mut self, st: &Stmt) {
        match st {
            Stmt::Let { ty, init, slot, .. } => {
                let vty = ty.clone().unwrap_or(Type::I64);
                if let Type::Array(elem, n) = &vty {
                    match init {
                        // `[v; n]` splats one value across every element. A
                        // scalar `v` broadcasts its word; an aggregate `v` is a
                        // copy of its bytes into each element.
                        Some(Expr::ArrayLit(v, _)) => {
                            self.gen_expr(v);
                            if elem.is_aggregate() {
                                self.splat_aggregate(*slot, elem.size(), *n);
                            } else {
                                self.fill_aggregate(*slot, elem.size(), *n);
                            }
                        }
                        // Anything else yields the array's address (a list
                        // literal desugars to a block ending in a local array),
                        // so copy the elements into the destination.
                        Some(e) => {
                            self.gen_expr(e);
                            self.copy_aggregate(*slot, *n, elem.size());
                        }
                        None => {
                            self.line("xor %eax, %eax");
                            self.fill_aggregate(*slot, elem.size(), *n);
                        }
                    }
                } else if let Type::EnumData(_, size) = &vty {
                    match init {
                        // A ctor, a call, another enum local: all yield the
                        // value's address, so copy its words in.
                        Some(e) => {
                            self.gen_expr(e);
                            self.copy_aggregate(*slot, *size / 8, 8);
                        }
                        None => {
                            self.line("xor %eax, %eax");
                            self.fill_aggregate(*slot, 8, *size / 8);
                        }
                    }
                } else if let Type::Struct(_, size) = &vty {
                    match init {
                        Some(Expr::StructLit { fields, .. }) => {
                            // The field-to-slot map uses the struct's slot
                            // count, not its field count: an array field spans
                            // several slots, so `fields.len() != size / 8`.
                            let base = self.array_off(*slot, size / 8);
                            let n = (size / 8) as i32;
                            for fi in fields {
                                let reg = self.struct_field_reg(*slot, n, fi.offset);
                                self.store_field_value(base + fi.offset, reg, fi);
                            }
                        }
                        // No initializer: the struct is zero-filled, into the
                        // register of each promoted field and memory otherwise.
                        None => {
                            let base = self.array_off(*slot, size / 8);
                            let n = (size / 8) as i32;
                            self.line("xor %eax, %eax");
                            for i in 0..n {
                                match self.reg_of(*slot + n - 1 - i) {
                                    Some(r) => self.line(&format!("xor {r}, {r}")),
                                    None => {
                                        self.line(&format!("mov %rax, {}(%rbp)", base + i * 8))
                                    }
                                }
                            }
                        }
                        // Any other initialiser yields the struct's address (a
                        // call's hidden return buffer, or another struct local),
                        // so copy it into the destination.
                        Some(e) => {
                            self.gen_expr(e);
                            self.copy_aggregate(*slot, size / 8, 8);
                        }
                    }
                } else if let Some(e) = init {
                    // Write straight into the target: the promoted register when
                    // there is one, else `%rax` for the frame store.
                    match self.reg_of(*slot) {
                        Some(r) => self.gen_expr_into_reg(e, r),
                        None => self.gen_expr_to_slot(e, *slot),
                    }
                } else {
                    match self.reg_of(*slot) {
                        Some(r) => self.line(&format!("xor {r}, {r}")),
                        None => self.line(&format!("movq $0, {}(%rbp)", self.off(*slot))),
                    }
                }
            }
            Stmt::Assign { target, value } => {
                match target {
                    Expr::Var { slot, ty, .. } if ty.is_aggregate() => {
                        // Whole-aggregate assignment copies the contents, not the
                        // source address, preserving by-value semantics.
                        let n = ty.size() / 8;
                        self.gen_expr(value);
                        self.line("mov %rax, %rsi");
                        self.line(&format!("lea {}(%rbp), %rdi", self.array_off(*slot, n)));
                        self.copy_words_rdi_rsi(n);
                    }
                    Expr::Var { slot, .. } => {
                        match self.reg_of(*slot) {
                            Some(r) => self.gen_expr_into_reg(value, r),
                            None => self.gen_expr_to_slot(value, *slot),
                        }
                    }
                    Expr::Field { ty, .. } if ty.is_aggregate() => {
                        self.assign_aggregate(target, value, ty.size() / 8);
                    }
                    Expr::Field { base, offset, .. } => {
                        if let Some(r) = self.field_reg(base, *offset) {
                            // The field lives in a promoted register. A register
                            // or immediate value folds into the move; anything
                            // else materialises the value, then moves it.
                            match self.simple_operand(value) {
                                Some(op) if op.starts_with('%') || op.starts_with('$') => {
                                    self.line(&format!("mov {op}, {r}"));
                                }
                                _ => self.gen_expr_into_reg(value, r),
                            }
                        } else {
                            match self.field_operand(base, *offset) {
                                Some(op) => {
                                    // `field += x` folds into one read-modify-write
                                    // `add`/`sub`, the shape of a running total.
                                    if self.gen_mem_accumulate(&op, value) {
                                        return;
                                    }
                                    match self.simple_operand(value) {
                                        Some(v) if v.starts_with('%') || v.starts_with('$') => {
                                            self.store_operand(&v, &op);
                                        }
                                        _ => {
                                            self.gen_expr(value);
                                            self.line(&format!("mov %rax, {op}"));
                                        }
                                    }
                                }
                                None => self.store_through_addr(target, value, Some(0)),
                            }
                        }
                    }
                    Expr::Index(base, idx)
                        if Self::array_elem_ty(base).map_or(false, |e| e.is_aggregate()) =>
                    {
                        // An aggregate element (`m[i] = [..]`, `a[i] = P{..}`) is
                        // copied word by word; `assign_aggregate` forms the
                        // element address, which scales by the element size.
                        let elem = Self::array_elem_ty(base).unwrap();
                        self.assign_aggregate(target, value, elem.size() / 8);
                    }
                    Expr::Index(base, idx) => {
                        match self.array_elem_operand(base, idx) {
                            // Constant index: one store.
                            Some(op) if !op.contains("%rax") => {
                                self.check_bounds(base, idx);
                                self.gen_expr(value);
                                self.line(&format!("mov %rax, {op}"));
                            }
                            // Variable index: load the index into %rax, then
                            // store through `disp(%rbp,%rax,8)`.
                            Some(op) => {
                                match self.simple_operand(value) {
                                    Some(vs) if vs.starts_with('$') => {
                                        self.gen_expr(idx);
                                        self.check_bounds(base, idx);
                                        self.line(&format!("movq {vs}, {op}"));
                                    }
                                    Some(vs) => {
                                        self.gen_expr(idx);
                                        self.check_bounds(base, idx);
                                        self.line(&format!("mov {vs}, %rcx"));
                                        self.line(&format!("mov %rcx, {op}"));
                                    }
                                    None => {
                                        self.gen_expr(value);
                                        self.push_rax();
                                        self.gen_expr(idx);
                                        self.check_bounds(base, idx);
                                        self.pop_to("%rcx");
                                        self.line(&format!("mov %rcx, {op}"));
                                    }
                                }
                            }
                            None => {
                                // The value is evaluated before the assignee
                                // place (see `store_through_addr`), and here the
                                // place's index is what may call, so the value
                                // must be stashed before the address is formed.
                                if Self::place_has_effect(target) {
                                    self.gen_expr(value);
                                    self.push_rax();
                                    self.gen_addr(target);
                                    self.line("mov %rax, %rcx");
                                    self.pop_to("%rax");
                                    self.line("mov %rax, (%rcx)");
                                } else {
                                    self.gen_addr(target);
                                    self.push_rax();
                                    self.gen_expr(value);
                                    self.pop_to("%rcx");
                                    self.line("mov %rax, (%rcx)");
                                }
                            }
                        }
                    }
                    // `*r = v`: the target's address is the pointer `r` holds.
                    Expr::Deref { ty, .. } if ty.is_aggregate() => {
                        self.assign_aggregate(target, value, ty.size() / 8);
                    }
                    Expr::Deref { .. } => {
                        self.store_through_addr(target, value, Some(0));
                    }
                    _ => unreachable!("sema guarantees a valid assignment target"),
                }
            }
            Stmt::Expr(e) => {
                self.gen_stmt_expr(e);
            }
            Stmt::While(cond, body, label) => {
                // Rotated: the test moves to the bottom and the body falls
                // through to it, so the back edge is the conditional branch and
                // the loop no longer pays an unconditional `jmp` every
                // iteration. The entry `jmp` runs once and reuses the same test,
                // so a false first condition still skips the body. `continue`
                // targets the test (re-check), `break` targets `end`, which the
                // test falls into.
                let body_l = self.new_label();
                let test_l = self.new_label();
                let end = self.new_label();
                self.loops.push((test_l.clone(), end.clone(), -1, label.clone()));
                self.line(&format!("jmp {test_l}"));
                // Align the loop body to a 16-byte fetch boundary so its
                // layout does not shift with unrelated code-size changes.
                self.raw("\t.p2align 4");
                self.raw(&format!("{body_l}:"));
                self.gen_block(body);
                self.raw(&format!("{test_l}:"));
                self.gen_branch_true(cond, &body_l);
                self.raw(&format!("{end}:"));
                self.loops.pop();
            }
            Stmt::For { var_slot, start, end, end_slot, inclusive, body, label, .. } => {
                // Evaluate both bounds once: `var = start`, `end_slot = end`.
                self.gen_expr_to_slot(start, *var_slot);
                // A constant bound needs no slot: it has no side effect, so it
                // can be compared against directly and the frame word (and its
                // reload every iteration) disappears. A non-constant bound is
                // evaluated once into its slot.
                let end_imm = Self::small_imm(end);
                if end_imm.is_none() {
                    self.gen_expr_to_slot(end, *end_slot);
                }
                // Rotated like `while`: the guard is the back edge, with an
                // entry `jmp` reusing it, so an iteration is one branch fewer
                // than a top-tested form. The increment still sits between the
                // body and the guard, so `continue` (which targets it) runs it
                // exactly once, and the entry `jmp` reaches the guard with the
                // un-incremented start.
                let top = self.new_label();
                let cont = self.new_label();
                let guard = self.new_label();
                let exit = self.new_label();
                // `continue` must still run the increment, so it targets `cont`;
                // `break` leaves past the guard.
                self.loops.push((cont.clone(), exit.clone(), -1, label.clone()));
                self.line(&format!("jmp {guard}"));
                self.raw("\t.p2align 4");
                self.raw(&format!("{top}:"));
                self.gen_block(body);
                self.raw(&format!("{cont}:"));
                // An inclusive loop must stop *before* incrementing past the
                // endpoint: `MAX..=MAX` would otherwise wrap the counter to
                // MIN and loop forever. Only a variable or `i64::MAX` endpoint
                // can reach the wrap, so a smaller constant bound keeps the
                // branch-free increment.
                if *inclusive && !matches!(end_imm, Some(k) if k != i64::MAX) {
                    self.gen_var_operand(*var_slot, "%rax");
                    match end_imm {
                        Some(k) => self.line(&format!("cmp ${k}, %rax")),
                        None => {
                            self.gen_var_operand(*end_slot, "%rcx");
                            self.line("cmp %rcx, %rax");
                        }
                    }
                    self.line(&format!("je {exit}"));
                }
                self.inc_slot(*var_slot);
                self.raw(&format!("{guard}:"));
                self.gen_var_operand(*var_slot, "%rax");
                match end_imm {
                    Some(k) => self.line(&format!("cmp ${k}, %rax")),
                    None => {
                        self.gen_var_operand(*end_slot, "%rcx");
                        self.line("cmp %rcx, %rax");
                    }
                }
                // Loop back while `var < end`, or while `var <= end` for `..=`.
                self.line(&format!("{} {top}", if *inclusive { "jle" } else { "jl" }));
                self.raw(&format!("{exit}:"));
                self.loops.pop();
            }
            Stmt::Return(v) => {
                match v {
                    Some(e) => {
                        self.gen_expr(e);
                        if self.ret_ty.is_aggregate() {
                            let n = self.ret_ty.size() / 8;
                            self.gen_return_aggregate_from_rax(n);
                        }
                    }
                    None => self.line("xor %eax, %eax"),
                }
                self.line(&format!("jmp {}", self.epilogue));
            }
            Stmt::Break(v, label) => {
                let i = self.loop_target(label);
                let (_, end, slot, _) = self.loops[i].clone();
                if let Some(e) = v {
                    // A unit- or `!`-valued loop has no result slot (`-1`), so
                    // there is nothing to store; writing `off(-1)` would hit the
                    // saved frame pointer. Still evaluate the expression for its
                    // side effects, as rustc does.
                    if slot < 0 {
                        self.gen_expr(e);
                    } else {
                        // Store the break value into the loop's result slot, then
                        // leave; the loop expression loads it after `end`.
                        self.gen_expr_to_slot(e, slot);
                    }
                }
                self.line(&format!("jmp {end}"));
            }
            Stmt::Continue(label) => {
                let i = self.loop_target(label);
                let top = self.loops[i].0.clone();
                self.line(&format!("jmp {top}"));
            }
        }
    }

    /// Load local `slot` (memory or a promoted register) into `reg`.
    fn gen_var_operand(&mut self, slot: i32, reg: &str) {
        match self.reg_of(slot) {
            Some(r) => self.line(&format!("mov {r}, {reg}")),
            None => self.line(&format!("mov {}(%rbp), {reg}", self.off(slot))),
        }
    }

    /// Copy the `n`-element aggregate whose address is in `%rax` into the
    /// aggregate starting at `slot`. Each element is `elem_size` bytes, so the
    /// copy moves `n * elem_size / 8` words. Used to initialise an array (from a
    /// list literal, or an element-wise copy) or a struct, which produce their
    /// address rather than their elements.
    fn copy_aggregate(&mut self, slot: i32, n: usize, elem_size: usize) {
        if n == 0 || elem_size == 0 {
            return;
        }
        let words = n * elem_size / 8;
        let base = self.array_off(slot, words);
        if words <= 8 {
            for i in 0..words {
                self.line(&format!("mov {}(%rax), %rdx", i as i32 * 8));
                self.line(&format!("mov %rdx, {}(%rbp)", base + i as i32 * 8));
            }
        } else {
            let l = self.new_label();
            self.line("mov %rax, %rsi");
            self.line(&format!("lea {base}(%rbp), %rdi"));
            self.line(&format!("mov ${words}, %rcx"));
            self.raw(&format!("{l}:"));
            self.line("mov (%rsi), %rdx");
            self.line("mov %rdx, (%rdi)");
            self.line("add $8, %rsi");
            self.line("add $8, %rdi");
            self.line("dec %rcx");
            self.line(&format!("jnz {l}"));
        }
    }

    /// Store `%rax` into the `n`-element aggregate starting at `slot`. Each
    /// element is `elem_size` bytes, so `n * elem_size / 8` words are filled.
    /// Used to zero-fill or splat an array or struct that lives in the frame.
    fn fill_aggregate(&mut self, slot: i32, elem_size: usize, n: usize) {
        if elem_size == 0 {
            return;
        }
        let words = n * elem_size / 8;
        let base = self.array_off(slot, words);
        if words == 0 {
            return;
        }
        if words <= 8 {
            for i in 0..words {
                let s = slot + words as i32 - 1 - i as i32;
                match self.reg_of(s) {
                    Some(r) => self.line(&format!("mov %rax, {r}")),
                    None => self.line(&format!("mov %rax, {}(%rbp)", base + i as i32 * 8)),
                }
            }
        } else {
            let l = self.new_label();
            self.line("mov %rax, %r8");
            self.line(&format!("mov ${words}, %rcx"));
            self.line(&format!("lea {base}(%rbp), %rdx"));
            self.raw(&format!("{l}:"));
            self.line("mov %r8, (%rdx)");
            self.line("add $8, %rdx");
            self.line("dec %rcx");
            self.line(&format!("jnz {l}"));
        }
    }

    /// Splat the aggregate of `elem_size` bytes at the address in `%rax` into
    /// each of `n` elements starting at `slot`. Used for `[v; n]` when `v` is
    /// itself an aggregate (a nested array or a struct), where every element is
    /// a copy of `v`'s bytes rather than a broadcast of one word.
    fn splat_aggregate(&mut self, slot: i32, elem_size: usize, n: usize) {
        let ew = elem_size / 8;
        if n == 0 || ew == 0 {
            return;
        }
        let words = n * ew;
        let base = self.array_off(slot, words);
        if words <= 8 {
            for i in 0..n {
                for w in 0..ew {
                    self.line(&format!("mov {}(%rax), %rdx", w as i32 * 8));
                    self.line(&format!(
                        "mov %rdx, {}(%rbp)",
                        base + (i * ew + w) as i32 * 8
                    ));
                }
            }
        } else {
            let l = self.new_label();
            self.line("mov %rax, %rsi");
            self.line(&format!("lea {base}(%rbp), %rdi"));
            self.line(&format!("mov ${n}, %rcx"));
            self.raw(&format!("{l}:"));
            for w in 0..ew {
                self.line(&format!("mov {}(%rsi), %r8", w as i32 * 8));
                self.line(&format!("mov %r8, {}(%rdi)", w as i32 * 8));
            }
            // Only the destination advances: every element is a copy of the
            // one source value, which stays put.
            self.line(&format!("add ${elem_size}, %rdi"));
            self.line("dec %rcx");
            self.line(&format!("jnz {l}"));
        }
    }

    /// The frame offset of element 0 / field 0 of an aggregate local, or `None`
    /// for a scalar. Aggregates occupy the low end of their slot range, so this
    /// is the *lowest* address of the range and fields index upward from it.
    fn aggregate_base(&self, slot: i32, ty: &Type) -> Option<i32> {
        match ty {
            Type::Array(elem, n) => {
                Some(self.array_off(slot, n * (elem.size() / 8).max(1)))
            }
            Type::Struct(_, size) => Some(self.array_off(slot, size / 8)),
            Type::EnumData(_, size) => Some(self.array_off(slot, size / 8)),
            _ => None,
        }
    }

    /// True when `e`'s type is a reference, so `e`'s value is a pointer and
    /// reaching a field or element through it uses that pointer directly.
    fn ref_base(e: &Expr) -> bool {
        match e {
            Expr::Var { ty, .. } | Expr::Field { ty, .. } | Expr::Deref { ty, .. } => {
                matches!(ty, Type::Ref(..))
            }
            _ => false,
        }
    }

    /// A `disp(base)` operand for `base.field`, or `None` when the base is not
    /// a form that can be addressed directly (the caller then computes the
    /// address). Two forms fold:
    ///
    /// * a struct local in memory: `disp(%rbp)`;
    /// * a pointer held in a register — a reference local, or a dereference of
    ///   one — which gives `disp(%reg)`, the whole `&self` field read as one
    ///   instruction instead of `mov`/`add`/`mov`.
    ///
    /// A reference held in memory (never promoted, or spilled) would need its
    /// pointer in a register first, so it does not fold here.
    fn field_operand(&self, base: &Expr, offset: i32) -> Option<String> {
        match base {
            Expr::Var { slot, ty: Type::Struct(_, size), .. } => {
                // The operand addresses field `offset`, which lives in slot
                // `slot + n - 1 - offset/8`. If that field was promoted, its
                // frame word is stale and the caller must use the register
                // instead, so decline and let it fall back.
                let fslot = *slot + (*size / 8) as i32 - 1 - offset / 8;
                if self.reg_of(fslot).is_some() {
                    return None;
                }
                Some(format!("{}(%rbp)", self.array_off(*slot, size / 8) + offset))
            }
            Expr::Var { slot, ty: Type::Ref(inner, _), .. } => {
                // A reference local's register holds its pointer, which is the
                // struct's address only when the referent *is* a struct. For a
                // `&&Struct` the referent is another pointer, so `r.field` must
                // load that inner pointer first (see the read path).
                match self.reg_of(*slot) {
                    Some(r) if inner.is_aggregate() => Some(format!("{}({r})", offset)),
                    _ => None,
                }
            }
            // `base` is `*expr`, whose value is the place `expr` points at. That
            // place is the struct address only when the deref's own type is the
            // aggregate; for `(*q).field` with `q: &&Struct` the deref yields a
            // pointer, whose address is the reference cell, not the struct.
            Expr::Deref { expr, ty } => match self.ptr_reg(expr) {
                Some(r) if ty.is_aggregate() => Some(format!("{}({r})", offset)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The register holding the pointer value of reference expression `e`, when
    /// `e` is a reference *local* kept in a register. This is the only form whose
    /// pointer needs no load: a `*p` used as a reference has its pointer in
    /// memory, so it is not a register operand.
    fn ptr_reg(&self, e: &Expr) -> Option<&'static str> {
        match e {
            Expr::Var { slot, ty: Type::Ref(..), .. } => self.reg_of(*slot),
            _ => None,
        }
    }

    /// The register holding the address of aggregate place `e`, when that
    /// address is already in a register: a reference local (`r.x` where `r: &T`)
    /// or a dereference of one (`(*q).x` where `q: &T`). A deeper chain
    /// (`(**q).x`) has its inner pointer in memory and does not fold.
    fn addr_reg(&self, e: &Expr) -> Option<&'static str> {
        match e {
            Expr::Var { slot, ty: Type::Ref(inner, _), .. } => {
                self.reg_of(*slot).filter(|_| inner.is_aggregate())
            }
            Expr::Deref { expr, ty } => {
                self.ptr_reg(expr).filter(|_| ty.is_aggregate())
            }
            _ => None,
        }
    }

    /// The register holding scalar field `offset` of struct local `base`, or
    /// `None` when the field stays in memory. Field 0 is the highest slot, so
    /// field `offset / 8` lives at `base + fields - 1 - offset / 8`.
    fn field_reg(&self, base: &Expr, offset: i32) -> Option<&'static str> {
        match base {
            Expr::Var { slot, ty: Type::Struct(_, size), .. } => {
                self.struct_field_reg(*slot, (*size / 8) as i32, offset)
            }
            _ => None,
        }
    }

    fn struct_field_reg(&self, slot: i32, fields: i32, offset: i32) -> Option<&'static str> {
        self.reg_of(slot + fields - 1 - offset / 8)
    }

    /// Stores `value` into a place whose address `gen_addr` computes. When the
    /// value is a register or immediate operand that no `gen_addr` clobbers,
    /// the store folds to one instruction (`mov $k, off(%reg)` or
    /// `mov %r, off(%reg)`) with no temporary slot. Otherwise it falls back to
    /// materialising the address, then the value, then storing through `%rcx`.
    /// `off` is a constant displacement added to the computed address (a
    /// variable index passes `None` and stays on the fallback path).
    fn store_through_addr(&mut self, target: &Expr, value: &Expr, off: Option<i32>) {
        // rustc evaluates the assigned value before the assignee place (the one
        // expression whose operands are ordered right-to-left). When computing
        // the place can run a call, evaluate the value first and stash it,
        // otherwise the place's call would be sequenced ahead of the value's.
        if Self::place_has_effect(target) {
            // The value goes first, but the place's address is what `gen_addr`
            // leaves in `%rax`; move it aside, reload the value into `%rax`, and
            // store the value through the address.
            self.gen_expr(value);
            self.push_rax();
            self.gen_addr(target);
            self.line("mov %rax, %rcx");
            self.pop_to("%rax");
            if is_byte_store(target) {
                self.line("movb %al, (%rcx)");
            } else {
                self.line("mov %rax, (%rcx)");
            }
            return;
        }
        if let (Some(op), Some(k)) = (self.simple_operand(value), off) {
            if (op.starts_with('%') || op.starts_with('$')) && !is_byte_store(target) {
                self.gen_addr(target);
                let disp = if k == 0 { String::new() } else { format!("{k}") };
                self.store_operand(&op, &format!("{disp}(%rax)"));
                return;
            }
        }
        self.gen_addr(target);
        self.push_rax();
        self.gen_expr(value);
        self.pop_to("%rcx");
        if is_byte_store(target) {
            self.line("movb %al, (%rcx)");
        } else {
            self.line("mov %rax, (%rcx)");
        }
    }

    /// Assign aggregate `value` (an address in `%rax`) into aggregate place
    /// `target` by copying `n` words. The value is evaluated before the place,
    /// as rustc orders assignment operands; when the place's address computation
    /// can call, the value is stashed across it. Otherwise the place is formed
    /// first and the value stashed across the copy's scratch use of `%rdi`.
    fn assign_aggregate(&mut self, target: &Expr, value: &Expr, n: usize) {
        if Self::place_has_effect(target) {
            self.gen_expr(value);
            self.push_rax();
            self.gen_addr(target);
            self.line("mov %rax, %rdi");
            self.pop_to("%rsi");
        } else {
            self.gen_addr(target);
            self.push_rax();
            self.gen_expr(value);
            self.line("mov %rax, %rsi");
            self.pop_to("%rdi");
        }
        self.copy_words_rdi_rsi(n);
    }

    /// Store register-or-immediate operand `op` into memory operand `dst`.
    ///
    /// An immediate store must be sized `movq`. `as` has no operand to infer a
    /// width from in `mov $imm, mem`, so it defaults to a 32-bit store and
    /// leaves the destination's upper four bytes untouched — a stale high half.
    /// A register operand names its own width, so a plain `mov` is already 64-bit.
    fn store_operand(&mut self, op: &str, dst: &str) {
        let mnemonic = if op.starts_with('$') { "movq" } else { "mov" };
        self.line(&format!("{mnemonic} {op}, {dst}"));
    }

    /// Store `e` straight into the memory operand `dst` when `e` is a small
    /// immediate, so `mov $k, %rax; mov %rax, mem` collapses to `movq $k, mem`.
    /// Store `e` straight into the memory operand `dst` when `e` is already a
    /// bare register or a small immediate, so `mov $k, %rax; mov %rax, mem`
    /// collapses to `movq $k, mem` and `mov %r, %rax; mov %rax, mem` to
    /// `mov %r, mem`. Returns false when `e` needs evaluating, so the caller
    /// falls back to evaluating it into `%rax`. A literal needs no overflow
    /// check and a register copy cannot trap.
    fn store_simple(&mut self, e: &Expr, dst: &str) -> bool {
        if let Some(k) = Self::small_imm(e) {
            self.store_operand(&format!("${k}"), dst);
            return true;
        }
        match self.simple_operand(e) {
            // A memory operand would make `mov mem, mem`, which is illegal.
            Some(op) if op.starts_with('%') => {
                self.line(&format!("mov {op}, {dst}"));
                true
            }
            _ => false,
        }
    }

    /// Leaves the address of an lvalue in `%rax`.
    fn gen_addr(&mut self, e: &Expr) {
        match e {
            Expr::Var { slot, ty, .. } => {
                let off = self.aggregate_base(*slot, ty).unwrap_or_else(|| self.off(*slot));
                self.line(&format!("lea {off}(%rbp), %rax"));
            }
            Expr::Index(base, idx) => {
                match base.as_ref() {
                    Expr::Var { slot, ty: Type::Array(elem, n), .. } => {
                        let slots = n * (elem.size() / 8).max(1);
                        self.line(&format!("lea {}(%rbp), %rax", self.array_off(*slot, slots)));
                    }
                    // A non-local base yields its address as a value.
                    _ => {
                        self.gen_expr(base);
                    }
                }
                self.push_rax();
                self.gen_expr(idx);
                self.check_bounds(base, idx);
                self.pop_to("%rcx");
                // Scale the index by the element size: 8 bytes for a scalar
                // element, more for a nested array or a struct.
                let stride = Self::elem_stride(base);
                if stride == 1 {
                    self.line("lea (%rcx,%rax,8), %rax");
                } else {
                    self.line(&format!("imul ${}, %rax, %rax", stride * 8));
                    self.line("lea (%rcx,%rax), %rax");
                }
            }
            Expr::Field { base, offset, .. } => {
                // A `Ref` base is a pointer: the address of its field is the
                // pointer value plus the offset, not the address of the slot
                // holding the pointer. When the pointer is already in a register
                // this is a single `lea`.
                if let Some(r) = self.addr_reg(base) {
                    self.line(&format!("lea {}({r}), %rax", *offset));
                } else {
                    if Self::ref_base(base) {
                        self.gen_expr(base);
                    } else {
                        self.gen_addr(base);
                    }
                    if *offset != 0 {
                        self.line(&format!("add ${}, %rax", *offset));
                    }
                }
            }
            // `*r` is a place whose address is the pointer `r` holds.
            Expr::Deref { expr, .. } => {
                self.gen_expr(expr);
            }
            // A field or index base that is not a named local is a temporary
            // whose value *is* its address: a call's hidden return buffer, a
            // block or `if` yielding a struct, or a struct literal. Evaluating
            // it leaves that address in `%rax`.
            Expr::Call { .. }
            | Expr::Block(_)
            | Expr::MatchBoundary(_, _)
            | Expr::Loop(..)
            | Expr::If(..)
            | Expr::StructLit { .. }
            | Expr::EnumCtor { .. } => {
                self.gen_expr(e);
            }
            // A payload field of aggregate type is named by its address, which
            // evaluating it yields.
            Expr::EnumPayload { ty, .. } if ty.is_aggregate() => {
                self.gen_expr(e);
            }
            _ => unreachable!("sema guarantees a valid address expression"),
        }
    }

    /// Generate `e` and, when its result is a narrow integer, truncate the
    /// 64-bit register to that width (sign- or zero-extended back to 64 bits).
    /// rustc keeps a `u8`/`u16`/`i32`/... value wrapped at its width, so a
    /// later use — printing with `{}`, a division, an ordering comparison, a
    /// cast to a wider type — must see the wrapped value, not the raw 64-bit
    /// arithmetic result. A 64-bit or non-integer result is left alone, so an
    /// `i64` program is unaffected.
    fn gen_expr(&mut self, e: &Expr) -> Type {
        let ty = self.gen_expr_raw(e);
        // The narrowing key is `static_ty`, which models a narrow arithmetic
        // result (`gen_expr_raw` reports a bare `i64` for a `Bin`, since the
        // emitted instruction is 64-bit).
        self.narrow_trunc(e, "%rax");
        ty
    }

    /// Truncate `reg` back to `e`'s narrow width when `e` is a genuine
    /// arithmetic result that may have wrapped past it. A literal, a variable
    /// load or a byte load is already in range, so only the forms `may_wrap`
    /// reports need this; the `lea`/`accumulate` fast paths in
    /// `gen_expr_into_reg` call this directly because they bypass `gen_expr`.
    fn narrow_trunc(&mut self, e: &Expr, reg: &str) {
        if let Some(st) = Self::static_ty(e) {
            if let Some(bits) = narrow_bits(&st) {
                if Self::may_wrap(e) {
                    self.truncate_reg(reg, bits, matches!(st, Type::Int { signed: true, .. }));
                }
            }
        }
    }

    /// True for the expression forms whose 64-bit result may exceed a narrow
    /// integer's range, so the value needs truncating. A literal, a variable, a
    /// field, a dereference, a call and a cast are already in range (or are the
    /// cast's own job); an `Index` of a narrow array element is too.
    fn may_wrap(e: &Expr) -> bool {
        match e {
            Expr::Bin(..) | Expr::Un(UnOp::Neg, _) | Expr::Un(UnOp::Not, _) => true,
            Expr::If(_, then, Some(els)) => {
                Self::may_wrap_tail(then) || Self::may_wrap_tail(els)
            }
            Expr::Block(b) | Expr::MatchBoundary(b, _) => Self::may_wrap_tail(b),
            _ => false,
        }
    }

    fn may_wrap_tail(b: &Block) -> bool {
        b.tail.as_ref().map(|t| Self::may_wrap(t)).unwrap_or(false)
    }

    /// Truncate `%rax` to the low `bits` and extend back to 64 bits, signed or
    /// unsigned. `bits` is 8, 16 or 32 (a narrower integer is not modelled).
    fn truncate_to(&mut self, bits: u16, signed: bool) {
        self.truncate_reg("%rax", bits, signed);
    }

    /// Truncate the 64-bit register `reg` to the low `bits` and extend back to
    /// 64 bits, signed or unsigned. `reg` must be a full 64-bit name (`%rax`,
    /// `%r12`); the sub-register names are derived from it.
    fn truncate_reg(&mut self, reg: &str, bits: u16, signed: bool) {
        let ext = match (bits, signed) {
            (8, true) => format!("movsbq {}, {reg}", reg8(reg)),
            (8, false) => format!("movzbq {}, {reg}", reg8(reg)),
            (16, true) => format!("movswq {}, {reg}", reg16(reg)),
            (16, false) => format!("movzwq {}, {reg}", reg16(reg)),
            (32, true) => format!("movslq {}, {reg}", r32(reg)),
            // A 32-bit value is already zero-extended in `reg` by any 32-bit
            // operation; a 64-bit result needs its high half cleared.
            (32, false) => format!("mov {}, {}", r32(reg), r32(reg)),
            _ => return,
        };
        self.line(&ext);
    }

    fn gen_expr_raw(&mut self, e: &Expr) -> Type {
        match e {
            Expr::ArrayList(_) => unreachable!("sema rewrites ArrayList into a block"),
            Expr::TupleLit(_) => unreachable!("sema rewrites TupleLit into a StructLit"),
            Expr::MethodCall { .. } => unreachable!("sema rewrites MethodCall into a Call"),
Expr::PatScrutinee(_) => {
                unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
            }
            Expr::PatCheck { .. } => unreachable!("sema rewrites PatCheck into `true`"),
            Expr::PatArity { .. } => unreachable!("sema rewrites PatArity into `true`"),
            Expr::EnumVariant { tag, .. } => {
                self.line(&format!("mov ${tag}, %rax"));
                Type::I64
            }
            Expr::EnumCtor { tag, args, arg_offsets, slot, size, enum_name, .. } => {
                // Materialise the value into its frame buffer and yield that
                // address, exactly like a struct literal: the tag word first,
                // then each argument at its field offset after the tag.
                let n = *size / 8;
                let base = self.array_off(*slot, n);
                self.line(&format!("movq ${tag}, {base}(%rbp)"));
                // Zero the payload first so a partially-initialised variant
                // never exposes stale frame bytes.
                for k in 1..n {
                    self.line(&format!("movq $0, {}(%rbp)", base + (k as i32) * 8));
                }
                for (i, a) in args.iter().enumerate() {
                    let off = 8 + arg_offsets[i];
                    // A payload field is always 8 bytes, so the store width is
                    // fixed. An immediate or a register can be stored directly;
                    // a memory operand (an unpromoted local, or a field) cannot,
                    // because `mov mem, mem` is not an x86 instruction — it must
                    // go through `%rax`.
                    match self.simple_operand(a) {
                        Some(op) if !op.contains('(') => {
                            // An immediate payload field needs the `movq` suffix:
                            // `as` reads an unsuffixed `mov $imm, mem` as a
                            // 32-bit store and leaves the field's high half stale.
                            self.store_operand(&op, &format!("{}(%rbp)", base + off));
                        }
                        _ => {
                        let at = self.gen_expr(a);
                        if at.is_aggregate() {
                            // An aggregate argument is a value named by its
                            // address; copy its words in.
                            let words = at.size() / 8;
                            self.line("mov %rax, %rsi");
                            self.line(&format!("lea {}(%rbp), %rdi", base + off));
                            self.copy_words_rdi_rsi(words);
                            } else {
                                self.line(&format!("mov %rax, {}(%rbp)", base + off));
                            }
                        }
                    }
                }
                self.line(&format!("lea {base}(%rbp), %rax"));
                Type::EnumData(enum_name.clone(), *size)
            }
            Expr::BoolTest { scrutinee, value } => {
                // The scrutinee is a bool word (0 or 1); compare it directly.
                self.gen_expr(scrutinee);
                self.line(&format!("cmp ${}, %rax", *value as i64));
                self.line("sete %al");
                self.line("movzbq %al, %rax");
                Type::Bool
            }
            Expr::VariantTest { scrutinee, tag, .. } => {
                // Compare the value's tag word. `gen_expr` yields the address
                // for an aggregate enum and the tag itself for a unit enum, so
                // branch on the returned type rather than on syntax.
                let st = self.gen_expr(scrutinee);
                if st.is_aggregate() {
                    self.line("mov (%rax), %rax");
                }
                self.line(&format!("cmp ${tag}, %rax"));
                self.line("sete %al");
                self.line("movzbq %al, %rax");
                Type::Bool
            }
            Expr::EnumPayload { recv, offset, ty, .. } => {
                // The receiver is a payload enum, so `gen_expr` yields its
                // address. A payload field is at `offset` (tag word included).
                self.gen_expr(recv);
                if ty.is_aggregate() {
                    if *offset != 0 {
                        self.line(&format!("add ${}, %rax", *offset));
                    }
                } else if let Some(r) = self.field_reg(recv, *offset) {
                    self.line(&format!("mov {r}, %rax"));
                } else {
                    self.line(&format!("mov {}(%rax), %rax", *offset));
                }
                ty.clone()
            }
            Expr::Int(v) => {
                self.line(&format!("mov ${v}, %rax"));
                Type::I64
            }
            Expr::Char(c) => {
                self.line(&format!("mov ${c}, %rax"));
                Type::Char
            }
            Expr::Bool(b) => {
                self.line(&format!("mov ${}, %rax", if *b { 1 } else { 0 }));
                Type::Bool
            }
            Expr::Str(s) => {
                let label = format!(".Lstr{}", self.str_id);
                self.str_id += 1;
                self.strings.push((label.clone(), s.clone()));
                self.line(&format!("lea {label}(%rip), %rax"));
                Type::I64
            }
            Expr::Var { slot, ty, .. } => {
                // A reference is a scalar: its value is the address it holds. An
                // aggregate is named by its own address.
                if ty.is_aggregate() {
                    match self.aggregate_base(*slot, ty) {
                        Some(base) => self.line(&format!("lea {base}(%rbp), %rax")),
                        None => self.line(&format!("mov {}(%rbp), %rax", self.off(*slot))),
                    }
                } else {
                    match self.reg_of(*slot) {
                        Some(r) => self.line(&format!("mov {r}, %rax")),
                        None => self.line(&format!("mov {}(%rbp), %rax", self.off(*slot))),
                    }
                }
                ty.clone()
            }
            Expr::Field { base, offset, ty, .. } => {
                if ty.is_aggregate() {
                    // A nested struct field is named by its address.
                    match self.field_operand(base, *offset) {
                        Some(op) => self.line(&format!("lea {op}, %rax")),
                        None => self.gen_addr(e),
                    }
                } else if let Some(r) = self.field_reg(base, *offset) {
                    self.line(&format!("mov {r}, %rax"));
                } else {
                    match self.field_operand(base, *offset) {
                        Some(op) => self.line(&format!("mov {op}, %rax")),
                        None => {
                            self.gen_addr(e);
                            self.line("mov (%rax), %rax");
                        }
                    }
                }
                ty.clone()
            }
            Expr::StructLit { name, fields, slot } => {
                // Materialise the literal into its frame buffer and yield that
                // address, which is how every aggregate value is represented.
                // Fields are stored in source order, so initializer side effects
                // run in the order they were written.
                // Slot count, which exceeds the field count when a field is
                // itself an aggregate (an array field spans several slots).
                let n = fields
                    .iter()
                    .map(|fi| fi.offset / 8 + (fi.ty.size() / 8) as i32)
                    .max()
                    .unwrap_or(0);
                let base = self.array_off(*slot, n as usize);
                for fi in fields {
                    let reg = self.struct_field_reg(*slot, n, fi.offset);
                    self.store_field_value(base + fi.offset, reg, fi);
                }
                self.line(&format!("lea {base}(%rbp), %rax"));
                Type::Struct(name.clone(), (n as usize) * 8)
            }
            Expr::Index(base, _idx) => {
                let elem = Self::array_elem_ty(base).unwrap_or(Type::I64);
                // An aggregate element is named by its address, not loaded: hand
                // the whole place to `gen_addr`, which scales by the element size.
                if elem.is_aggregate() {
                    self.gen_addr(e);
                    return elem;
                }
                let stride = (elem.size() / 8).max(1);
                match self.array_elem_operand(base, _idx) {
                    // `disp(%rbp,%reg,8)` addresses the element in one operand.
                    Some(op) if op.contains("%rax") => {
                        self.gen_expr(_idx);
                        self.check_bounds(base, _idx);
                        self.line(&format!("mov {op}, %rax"));
                    }
                    Some(op) => {
                        self.check_bounds(base, _idx);
                        self.line(&format!("mov {op}, %rax"));
                    }
                    None => {
                        // The base is not a local array of 8-byte elements: it is
                        // a temporary whose value *is* its address (a call's
                        // hidden return buffer, a block, a struct field), or a
                        // local array of wider elements, so index off the address
                        // with the element stride applied.
                        match _idx.as_ref() {
                            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => {
                                // Evaluate the base before checking, so a call or
                                // block base runs its side effects (Rust evaluates
                                // the base and index before the bounds check). The
                                // check uses no register, so the base address
                                // survives in `%rax` for the load.
                                self.gen_expr(base);
                                self.check_bounds(base, _idx);
                                self.line(&format!(
                                    "mov {}(%rax), %rax",
                                    *v as i32 * 8 * stride as i32
                                ));
                            }
                            _ => {
                                self.gen_expr(base);
                                self.push_rax();
                                self.gen_expr(_idx);
                                self.check_bounds(base, _idx);
                                self.pop_to("%rcx");
                                if stride == 1 {
                                    self.line("mov (%rcx,%rax,8), %rax");
                                } else {
                                    self.line(&format!("imul ${}, %rax, %rax", stride * 8));
                                    self.line("mov (%rcx,%rax), %rax");
                                }
                            }
                        }
                    }
                }
                elem
            }
            Expr::AddrOf { expr, ty, slot, mutable } => {
                if *slot >= 0 {
                    // The operand has no address of its own (a literal, a call,
                    // an aggregate value): evaluate it into a temporary and take
                    // that address. An aggregate value *is* its address.
                    let n = (ty.size() / 8).max(1);
                    self.gen_expr(expr);
                    if ty.is_aggregate() {
                        self.line("mov %rax, %rsi");
                        self.line(&format!("lea {}(%rbp), %rdi", self.array_off(*slot, n)));
                        self.copy_words_rdi_rsi(n);
                    } else {
                        self.line(&format!("mov %rax, {}(%rbp)", self.off(*slot)));
                    }
                    self.line(&format!("lea {}(%rbp), %rax", self.array_off(*slot, n)));
                } else {
                    self.gen_addr(expr);
                }
                Type::Ref(Box::new(ty.clone()), *mutable)
            }
            Expr::Deref { expr, ty } => {
                // A dereference is a place: read it. An aggregate referent is
                // named by its address, so reading it yields the pointer itself.
                self.gen_expr(expr);
                match byte_load(ty) {
                    Some(LoadKind::Zero) => self.line("movzbq (%rax), %rax"),
                    Some(LoadKind::Sign) => self.line("movsbq (%rax), %rax"),
                    None if !ty.is_aggregate() => self.line("mov (%rax), %rax"),
                    None => {}
                }
                // The value is an `i64` even for a byte referent, which is what
                // makes `*p` usable in printing and arithmetic.
                byte_value_ty(ty)
            }
            Expr::Un(op, inner) => {
                let inner_ty = self.gen_expr(inner);
                // `!` is logical not on `bool` (1 when the operand is zero)
                // and bitwise not on an integer, matching Rust. Sema has
                // already checked the operand is `bool` or `i64`. The operand's
                // type is taken from `static_ty` where it is known, because a
                // `&`/`|`/`^` on `bool` makes `gen_expr` report `i64` (the
                // emitted instruction is the same) and `!(a | b)` would
                // otherwise be a bitwise `not`. For a node `static_ty` does not
                // model (an index, say) `gen_expr`'s type is exact.
                let inner_is_bool = match Self::static_ty(inner) {
                    Some(t) => t == Type::Bool,
                    None => inner_ty == Type::Bool,
                };
                match op {
                    UnOp::Neg => {
                        self.line("neg %rax");
                        self.check_overflow();
                    }
                    UnOp::Not if inner_is_bool => {
                        self.line("test %rax, %rax");
                        self.line("sete %al");
                        self.line("movzbq %al, %rax");
                    }
                    UnOp::Not => {
                        self.line("not %rax");
                    }
                }
                match op {
                    UnOp::Neg => Type::I64,
                    UnOp::Not => inner_ty,
                }
            }
            Expr::Bin(op, a, b) => self.gen_bin(*op, a, b),
            Expr::Call { name, args, arg_slots, ret_slot, ret_ty } => {
                self.gen_call(name, args, arg_slots, *ret_slot, ret_ty)
            }
            Expr::If(cond, then, els) => {
                let else_l = self.new_label();
                let end_l = self.new_label();
                // An `if` without `else` in statement position produces no
                // value, so it needs neither the `xor` that materialises unit
                // nor a `jmp` over an empty else. The `then` block falls
                // straight through to the branch target.
                let bare = self.discard && els.is_none();
                // `if c { break; }` (or `continue;`) as the last statement of a
                // loop body: branching to `else` to reach an unconditional
                // `jmp` to the loop's exit costs two jumps. Branch to the exit
                // directly on the *negated* condition instead, so the taken path
                // is one jump and the fall-through path is none. This is the
                // escape test in `mandel`'s inner loop.
                if self.discard {
                    if let Some(dst) = self.bare_exit(then, els) {
                        self.gen_branch_true(cond, &dst);
                        return Type::Unit;
                    }
                }
                self.gen_branch_false(cond, &else_l);
                let tt = self.gen_block(then);
                if !bare {
                    self.line(&format!("jmp {end_l}"));
                }
                self.raw(&format!("{else_l}:"));
                let et = match els {
                    Some(e) => self.gen_block(e),
                    None => {
                        if !bare {
                            self.line("xor %eax, %eax");
                        }
                        Type::Unit
                    }
                };
                self.raw(&format!("{end_l}:"));
                if tt == Type::Unit { et } else { tt }
            }
            Expr::Loop(body, slot, lty, label) => {
                self.gen_loop_expr(body, *slot, label);
                lty.clone()
            }
            Expr::Block(b) => self.gen_block(b),
            Expr::MatchBoundary(b, _) => self.gen_block(b),
            Expr::ArrayLit(v, n) => {
                // Only meaningful as a `let` initializer; produce the value.
                let _ = n;
                self.gen_expr(v);
                Type::I64
            }
            Expr::Cast(inner, to) => {
                self.gen_expr(inner);
                // Normalise a cast to `bool` to 0/1, so a non-canonical value
                // such as `2 as bool` compares equal to `true` everywhere.
                if *to == Type::Bool {
                    self.line("test %rax, %rax");
                    self.line("setne %al");
                    self.line("movzbq %al, %rax");
                } else if let Some(bits) = narrow_bits(to) {
                    // `x as u8` keeps only the low byte; `x as i32` sign-extends
                    // the low word. A cast to a 64-bit type is a no-op on the
                    // register.
                    self.truncate_to(bits, matches!(to, Type::Int { signed: true, .. }));
                }
                to.clone()
            }
        }
    }

    /// An operand usable directly in an instruction without touching any
    /// register: an immediate, a scalar local, or an array element with a
    /// constant index. Anything needing a register (a variable index) returns
    /// `None` so the caller falls back to materialising the value in `%rax`.
    /// A local promoted to a register is returned as that register name.
    fn simple_operand(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => Some(format!("${v}")),
            Expr::Char(c) if *c <= 2_147_483_647 => Some(format!("${c}")),
            Expr::Bool(b) => Some(format!("${}", if *b { 1 } else { 0 })),
            Expr::Var { slot, ty, .. } => {
                if ty.is_aggregate() {
                    return None;
                }
                match self.reg_of(*slot) {
                    Some(r) => Some(r.to_string()),
                    None => Some(format!("{}(%rbp)", self.off(*slot))),
                }
            }
            Expr::Field { base, offset, ty, .. } => {
                if ty.is_aggregate() {
                    None
                } else if let Some(r) = self.field_reg(base, *offset) {
                    Some(r.to_string())
                } else {
                    self.field_operand(base, *offset)
                }
            }
            // `*p` reading a scalar through a reference in a register is a bare
            // memory operand, so `x + *p` folds into one `add`. An aggregate
            // referent's value is its address, which is not a simple operand.
            Expr::Deref { expr, ty } if !ty.is_aggregate() && byte_load(ty).is_none() => {
                self.ptr_reg(expr).map(|r| format!("({r})"))
            }
            Expr::Index(base, idx) => {
                // With bounds checks on, an element is never used as a bare
                // operand: it is materialised through `gen_expr`, which emits
                // the check.
                if self.bounds {
                    return None;
                }
                match self.array_elem_operand(base, idx) {
                    // Only register-free operands are usable here.
                    Some(op) if !op.contains("%rax") => Some(op),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Store `%rax` into local `slot`, honouring a register promotion.
    /// Emit a jump to the runtime trap if the flags show signed overflow. Only
    /// called when overflow checks are enabled.
    fn check_overflow(&mut self) {
        if self.overflow {
            self.line("jo __rmc_overflow");
        }
    }

    /// Trap if `reg` is not a valid shift count (0..=63). The hardware masks a
    /// shift count to 6 bits, so without this a count of 81 silently shifts by
    /// 17; Rust's `<<`/`>>` panics instead. Only called when overflow checks are
    /// enabled.
    fn check_shift_count(&mut self, reg: &str) {
        if self.overflow {
            self.line(&format!("cmp $63, {reg}"));
            self.line("ja __rmc_shift_overflow");
        }
    }

    /// Store one struct-literal field value at `dest_off(%rbp)`. A scalar field
    /// is a single word store; an aggregate field is copied word by word, since
    /// the value expression yields the field's *address*, not its contents.
    fn store_field_value(&mut self, dest_off: i32, reg: Option<&'static str>, fi: &FieldInit) {
        if fi.ty.is_aggregate() {
            self.gen_expr(&fi.value);
            let n = fi.ty.size() / 8;
            self.line("mov %rax, %rsi");
            self.line(&format!("lea {dest_off}(%rbp), %rdi"));
            self.copy_words_rdi_rsi(n);
            return;
        }
        // A promoted field's initialiser is emitted straight into its register,
        // and an unpromoted constant straight into its frame slot, so neither
        // bounces through `%rax` first.
        if let Some(r) = reg {
            self.gen_expr_into_reg(&fi.value, r);
            return;
        }
        if self.store_simple(&fi.value, &format!("{dest_off}(%rbp)")) {
            return;
        }
        self.gen_expr(&fi.value);
        self.line(&format!("mov %rax, {dest_off}(%rbp)"));
    }

    /// Evaluate `e` into local `slot`: the promoted register when there is one,
    /// else its frame word, folding a bare register/immediate initialiser
    /// straight into the store so no `%rax` bounce is emitted.
    fn gen_expr_to_slot(&mut self, e: &Expr, slot: i32) {
        match self.reg_of(slot) {
            Some(r) => self.gen_expr_into_reg(e, r),
            None => {
                let dst = format!("{}(%rbp)", self.off(slot));
                if !self.store_simple(e, &dst) {
                    self.gen_expr_into_reg(e, "%rax");
                    self.line(&format!("mov %rax, {dst}"));
                }
            }
        }
    }

    /// Add one to local `slot` in place, without routing through `%rax`.
    fn inc_slot(&mut self, slot: i32) {
        match self.reg_of(slot) {
            Some(r) => self.line(&format!("add $1, {r}")),
            None => self.line(&format!("addq $1, {}(%rbp)", self.off(slot))),
        }
    }

    /// Materialise `e` directly into `dst` (a register), avoiding a round trip
    /// through `%rax` where the destination is already known. `x + k` / `x - k`
    /// for a register `x` and a small constant becomes a single `lea`, which is
    /// the common case in argument setup and in `let t = i - 1;`.
    fn gen_expr_into_reg(&mut self, e: &Expr, dst: &str) {
        // A folded `lea` cannot trap, so the strength reductions below are only
        // sound when overflow checks are off; with them on, fall through to
        // `gen_expr`, which emits the `jo` after each arithmetic op.
        if !self.overflow {
            if let Some((reg, k, c)) = self.lea_shape(e) {
                // `reg` alone is a plain move; anything else is one `lea`.
                if k != 1 || c != 0 {
                    self.line(&format!("lea {}, {dst}", Self::lea_operand(&reg, k, c)));
                    self.narrow_trunc(e, dst);
                    return;
                }
            }
            if self.gen_accumulate(e, dst) {
                self.narrow_trunc(e, dst);
                return;
            }
        }
        if let Some(op) = self.simple_operand(e) {
            if op != dst {
                self.line(&format!("mov {op}, {dst}"));
            }
            return;
        }
        self.gen_expr(e);
        if dst != "%rax" {
            self.line(&format!("mov %rax, {dst}"));
        }
    }

    /// Fold `field += a - b + ...` into a run of read-modify-write `add`/`sub`
    /// against the field's memory operand, when the chain's base term is the
    /// field itself. The generic path loads the field, evaluates the whole sum
    /// in registers and stores it back; this keeps the running total in memory
    /// and touches each term once, which is the shape of an accumulator or a
    /// counter field in a hot loop. Only a register or immediate term folds
    /// into the instruction: a memory term would need `add mem, mem`, which is
    /// not an instruction, and could alias the field being written, so such a
    /// value falls back to the generic path. Overflow checks are also left to
    /// the generic path, where each `add` gets its own `jo`.
    fn gen_mem_accumulate(&mut self, op: &str, value: &Expr) -> bool {
        if self.overflow {
            return false;
        }
        let mut terms: Vec<(bool, &Expr)> = Vec::new();
        Self::flatten_chain(value, false, &mut terms);
        let (base_sub, first) = terms[0];
        let first_is_target = matches!(
            first,
            Expr::Field { base, offset, .. }
                if self.field_operand(base, *offset).as_deref() == Some(op)
        );
        if base_sub || !first_is_target {
            return false;
        }
        // Resolve every term before emitting anything, so a term that cannot
        // fold bails out without a half-written chain.
        let mut plan: Vec<(bool, String)> = Vec::new();
        for (is_sub, term) in &terms[1..] {
            match self.simple_operand(term) {
                Some(src) if !src.contains('(') => plan.push((*is_sub, src)),
                _ => return false,
            }
        }
        for (is_sub, src) in plan {
            let m = if is_sub { "sub" } else { "add" };
            // An immediate operand needs an explicit 64-bit width: `as` reads a
            // suffixless `add $imm, mem` as a 32-bit add, which would truncate.
            // A register operand names its own width, so it needs no suffix.
            let m = if src.starts_with('$') { format!("{m}q") } else { m.to_string() };
            self.line(&format!("{m} {src}, {op}"));
        }
        true
    }

    /// Fold `dst = dst + a + b - c + ...` into one `add`/`sub` per term, when
    /// `dst` already holds the chain's leftmost operand (its own previous
    /// value). A term that is already a single operand (an immediate, a
    /// register, or a memory reference) folds straight into the instruction; a
    /// term that needs evaluating (a call, a product) is computed into `%rax`
    /// and added with one `add %rax, dst`. Either way this replaces the generic
    /// expression tree, which bounces every partial result through `%rax` and
    /// the spill slots, with a flat run of `add`s and `sub`s on the
    /// accumulator's own register — the shape of a running sum.
    ///
    /// The chain is flattened first, so a nested sum counts: `acc + a + (b + c)`
    /// becomes three adds rather than evaluating `b + c` in `%rax` and adding it
    /// once. That case is common after LICM, which hoists each invariant term
    /// into its own local and can leave the surviving expression as a sum of
    /// sums.
    ///
    /// Returns false when `e` is not such a chain or when a term reads the
    /// accumulator register itself: `dst` is updated in place, so a term that
    /// read the *original* value would see the running result instead. The
    /// evaluated-term form is skipped under `--overflow-checks`, where that
    /// `add` would need its own `jo`. Each case falls back to the tree, which
    /// keeps `dst` untouched until the end.
    fn gen_accumulate(&mut self, e: &Expr, dst: &str) -> bool {
        let mut terms: Vec<(bool, &Expr)> = Vec::new(); // (is_sub, operand)
        Self::flatten_chain(e, false, &mut terms);
        // The first term is the chain's base and must be `dst` itself.
        let (base_sub, base) = terms[0];
        if terms.len() < 2 || base_sub || self.reg_operand(base).as_deref() != Some(dst) {
            return false;
        }
        if terms[1..].iter().any(|(_, x)| self.uses_reg(x, dst)) {
            return false;
        }
        // Resolve every term before emitting anything, so a term that cannot be
        // folded bails out without a half-written chain. A simple operand folds
        // straight into the `add`/`sub`; a term that must be evaluated (a call,
        // a product) is computed into `%rax` and added with a single `add
        // %rax, dst`, saving the `%rax`->`%rcx`->`%rax` bounce. The evaluated
        // form is only sound with overflow checks off — it has no `jo` — and
        // `%rax` is free between terms because no term may read `dst` and the
        // scratch registers never include a promoted local.
        enum Term<'a> {
            Simple(String),
            Eval(&'a Expr),
        }
        let mut plan: Vec<(bool, Term)> = Vec::new();
        for (is_sub, other) in &terms[1..] {
            if let Some(op) = self.simple_operand(other) {
                plan.push((*is_sub, Term::Simple(op)));
            } else if !self.overflow {
                plan.push((*is_sub, Term::Eval(other)));
            } else {
                return false;
            }
        }
        for (is_sub, term) in plan {
            let mnemonic = if is_sub { "sub" } else { "add" };
            match term {
                Term::Simple(op) => self.line(&format!("{mnemonic} {op}, {dst}")),
                Term::Eval(x) => {
                    self.gen_expr(x);
                    self.line(&format!("{mnemonic} %rax, {dst}"));
                }
            }
        }
        true
    }

    /// Flatten a `+`/`-` tree into signed terms, left to right. Subtraction
    /// flips the sign of its right subtree, so `x - (a - b)` yields `x`, `-a`,
    /// `+b`. Anything that is not an add or a subtract is a single term.
    fn flatten_chain<'a>(e: &'a Expr, sub: bool, out: &mut Vec<(bool, &'a Expr)>) {
        match e {
            Expr::Bin(BinOp::Add, a, b) => {
                Self::flatten_chain(a, sub, out);
                Self::flatten_chain(b, sub, out);
            }
            Expr::Bin(BinOp::Sub, a, b) => {
                Self::flatten_chain(a, sub, out);
                Self::flatten_chain(b, !sub, out);
            }
            _ => out.push((sub, e)),
        }
    }

    /// True when evaluating `e` reads register `r`, the storage of a promoted
    /// local. Only a `Var` or `Field` whose slot is promoted to `r` reads it
    /// directly; every other node is checked by recursing into its operands.
    fn uses_reg(&self, e: &Expr, r: &str) -> bool {
        if self.reg_operand(e).as_deref() == Some(r) {
            return true;
        }
        match e {
            Expr::Int(_) | Expr::Bool(_) | Expr::Char(_) | Expr::Str(_) | Expr::EnumVariant { .. } => false,
            Expr::Var { .. } => false,
            Expr::Field { base, .. } => self.uses_reg(base, r),
            Expr::Bin(_, a, b) => self.uses_reg(a, r) || self.uses_reg(b, r),
            Expr::Un(_, a) | Expr::Cast(a, _) => self.uses_reg(a, r),
            Expr::Index(a, b) => self.uses_reg(a, r) || self.uses_reg(b, r),
            Expr::Call { args, .. } => args.iter().any(|a| self.uses_reg(a, r)),
            Expr::AddrOf { expr, .. } | Expr::Deref { expr, .. } => self.uses_reg(expr, r),
            Expr::StructLit { fields, .. } => fields.iter().any(|f| self.uses_reg(&f.value, r)),
            Expr::ArrayLit(v, _) => self.uses_reg(v, r),
            Expr::EnumCtor { args, .. } => args.iter().any(|a| self.uses_reg(a, r)),
            Expr::VariantTest { scrutinee, .. } => self.uses_reg(scrutinee, r),
            Expr::BoolTest { scrutinee, .. } => self.uses_reg(scrutinee, r),
            Expr::EnumPayload { recv, .. } => self.uses_reg(recv, r),
            Expr::If(c, t, els) => {
                self.uses_reg(c, r)
                    || self.block_uses_reg(t, r)
                    || els.as_ref().map_or(false, |b| self.block_uses_reg(b, r))
            }
            Expr::Block(b) | Expr::Loop(b, _, _, _) => self.block_uses_reg(b, r),
            Expr::MatchBoundary(b, _) => self.block_uses_reg(b, r),
            Expr::ArrayList(_) | Expr::TupleLit(_) | Expr::MethodCall { .. } => {
                unreachable!("sema rewrites ArrayList, TupleLit and MethodCall")
            }
Expr::PatScrutinee(_) => {
                unreachable!("sema rewrites PatScrutinee into a Deref or the bare scrutinee")
            }
            Expr::PatCheck { .. } => unreachable!("sema rewrites PatCheck into `true`"),
            Expr::PatArity { .. } => unreachable!("sema rewrites PatArity into `true`"),
        }
    }

    fn block_uses_reg(&self, b: &Block, r: &str) -> bool {
        b.stmts.iter().any(|s| self.stmt_uses_reg(s, r))
            || b.tail.as_deref().map_or(false, |e| self.uses_reg(e, r))
    }

    fn stmt_uses_reg(&self, s: &Stmt, r: &str) -> bool {
        match s {
            Stmt::Let { init, .. } => init.as_ref().map_or(false, |e| self.uses_reg(e, r)),
            Stmt::Assign { target, value } => {
                self.uses_reg(target, r) || self.uses_reg(value, r)
            }
            Stmt::Expr(e) => self.uses_reg(e, r),
            Stmt::While(c, b, _) => self.uses_reg(c, r) || self.block_uses_reg(b, r),
            Stmt::For { start, end, body, .. } => {
                self.uses_reg(start, r)
                    || self.uses_reg(end, r)
                    || self.block_uses_reg(body, r)
            }
            Stmt::Return(v) => v.as_ref().map_or(false, |e| self.uses_reg(e, r)),
            Stmt::Break(v, _) => v.as_ref().map_or(false, |e| self.uses_reg(e, r)),
            Stmt::Continue(_) => false,
        }
    }

    /// The register holding `e` when it is a scalar local or field promoted to
    /// a register. `lea` needs a register base/index, so memory operands and
    /// immediates do not qualify.
    fn reg_operand(&self, e: &Expr) -> Option<String> {
        self.simple_operand(e).filter(|s| s.starts_with('%'))
    }

    /// Recognise `k * x + c` where `x` is register-resident and `k` is one of
    /// the scales an `lea` can express, returning `(x, k, c)`. This folds
    /// `2*i`, `3*i + 1`, `8*i` and friends into a single `lea`.
    fn lea_shape(&self, e: &Expr) -> Option<(String, i64, i64)> {
        if let Expr::Bin(op, a, b) = e {
            match op {
                BinOp::Add => {
                    if let Some(c) = Self::small_imm(b) {
                        if let Some((r, k, c0)) = self.lea_shape(a) {
                            if Self::disp_ok(c0 + c) {
                                return Some((r, k, c0 + c));
                            }
                        }
                    }
                    if let Some(c) = Self::small_imm(a) {
                        if let Some((r, k, c0)) = self.lea_shape(b) {
                            if Self::disp_ok(c0 + c) {
                                return Some((r, k, c0 + c));
                            }
                        }
                    }
                }
                BinOp::Sub => {
                    if let Some(c) = Self::small_imm(b) {
                        if let Some((r, k, c0)) = self.lea_shape(a) {
                            if Self::disp_ok(c0 - c) {
                                return Some((r, k, c0 - c));
                            }
                        }
                    }
                }
                BinOp::Mul => {
                    let (k, x) = match (Self::small_imm(a), Self::small_imm(b)) {
                        (Some(k), None) => (k, b),
                        (None, Some(k)) => (k, a),
                        _ => return None,
                    };
                    if matches!(k, 1 | 2 | 3 | 4 | 5 | 8 | 9) {
                        if let Some(r) = self.reg_operand(x) {
                            return Some((r, k, 0));
                        }
                    }
                }
                _ => {}
            }
        }
        self.reg_operand(e).map(|r| (r, 1, 0))
    }

    /// The scale for `x * k` when `k` is positive and expressible as a `lea`
    /// scale (`base + index*scale`), or `None`. `1` means the multiply is the
    /// identity and can be dropped.
    fn lea_mul_scale(k: i64) -> Option<i64> {
        (matches!(k, 1 | 2 | 3 | 4 | 5 | 8 | 9)).then_some(k)
    }

    /// The `lea` source operand computing `k * reg + c`, for the scales `lea`
    /// can express: 1, 2, 3, 4, 5, 8 and 9 (as base + index*scale).
    fn lea_operand(reg: &str, k: i64, c: i64) -> String {
        match k {
            1 => format!("{c}({reg})"),
            2 => format!("{c}({reg},{reg})"),
            3 => format!("{c}({reg},{reg},2)"),
            4 => format!("{c}(,{reg},4)"),
            5 => format!("{c}({reg},{reg},4)"),
            8 => format!("{c}(,{reg},8)"),
            9 => format!("{c}({reg},{reg},8)"),
            _ => unreachable!("lea_shape only yields expressible scales"),
        }
    }

    fn gen_bin(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Type {
        use BinOp::*;
        // `wrapping_mul` is `*` with no overflow guard. Run the whole `Mul`
        // path with the guard suppressed and the trap-restoring `jo` turned
        // off, so the constant folds, the `lea` scaling and the operand
        // selection are all shared and only the `jo` differs.
        if op == WrappingMul {
            let saved = self.overflow;
            self.overflow = false;
            let t = self.gen_bin(Mul, a, b);
            self.overflow = saved;
            return t;
        }
        match op {
            And | Or => {
                let short = self.new_label();
                let end = self.new_label();
                self.gen_expr(a);
                self.line("test %rax, %rax");
                match op {
                    And => {
                        self.line(&format!("jz {short}"));
                        self.gen_expr(b);
                        self.line("test %rax, %rax");
                        self.line("setne %al");
                        self.line("movzbq %al, %rax");
                        self.line(&format!("jmp {end}"));
                        self.raw(&format!("{short}:"));
                        self.line("xor %eax, %eax");
                    }
                    Or => {
                        self.line(&format!("jnz {short}"));
                        self.gen_expr(b);
                        self.line("test %rax, %rax");
                        self.line("setne %al");
                        self.line("movzbq %al, %rax");
                        self.line(&format!("jmp {end}"));
                        self.raw(&format!("{short}:"));
                        self.line("mov $1, %rax");
                    }
                    _ => unreachable!(),
                }
                self.raw(&format!("{end}:"));
                return Type::Bool;
            }
            _ => {}
        }

        // Comparisons: a direct `cmp`/`setcc` pair. When one side is a
        // register-free operand the comparison needs no stack traffic at all.
        if matches!(op, Eq | Ne | Lt | Le | Gt | Ge) {
            if !self.emit_cmp(a, b) {
                self.eval_operands(a, b, false);
                self.line("cmp %rcx, %rax");
            }
            self.line(&format!("{} %al", setcc(op)));
            self.line("movzbq %al, %rax");
            return Type::Bool;
        }

        // Division/modulo by a constant becomes a multiply, an arithmetic
        // shift and a sign fixup instead of a ~30-cycle `idiv`; by a variable
        // there is no shortcut, since the dividend must be in `%rax` and the
        // divisor in a register.
        if matches!(op, Div | Mod) {
            if self.gen_div_const(op, a, b) {
                return Type::I64;
            }
            // A promoted-local divisor is a callee-saved register, so `idiv`
            // can read it directly instead of bouncing it into `%rcx`. The
            // dividend still has to reach `%rax`, but `gen_expr(a)` uses only
            // `%rax`/`%rcx`/`%rdx` as scratch and `cqto` only writes `%rdx`, so
            // the divisor register stays live across both.
            if let Some(rb) = self.simple_operand(b) {
                if rb.starts_with('%') {
                    self.gen_expr(a);
                    self.emit_div(op, &rb, r32(&rb));
                    return Type::I64;
                }
            }
            self.eval_operands(a, b, false);
            self.emit_div(op, "%rcx", "%ecx");
            return Type::I64;
        }

        // Add/Sub/Mul: keep the left operand in `%rax` and the right in `%rcx`.
        // A small constant operand folds straight into the instruction, which
        // saves the constant load. Folding a *memory* operand into `imul`/`add`
        // measured slower on the cores this was tuned on, so only immediates
        // are folded.
        let (imm, other) = match Self::small_imm(b) {
            Some(k) => (Some(k), a),
            // Commutative ops accept the constant on either side; subtraction
            // and the shifts do not, because `c - x` is not `x - c` and
            // `c << x` is not `x << c`.
            None if matches!(op, Add | Mul | BitAnd | BitOr | BitXor) => match Self::small_imm(a) {
                Some(k) => (Some(k), b),
                None => (None, a),
            },
            None => (None, a),
        };
        if let Some(k) = imm {
            self.gen_expr(other);
            match op {
                Add => {
                    self.line(&format!("add ${k}, %rax"));
                    self.check_overflow();
                }
                Sub => {
                    self.line(&format!("sub ${k}, %rax"));
                    self.check_overflow();
                }
                Mul => {
                    // `x * k` for a `lea`-expressible `k` is one 1-cycle `lea`;
                    // `imul $k` is 3 cycles. This needs the same overflow-checks
                    // guard as the `lea_shape` fold above, because `lea` sets no
                    // flags and `imul`'s `OF` is what the trap test reads.
                    if !self.overflow {
                        match Self::lea_mul_scale(k) {
                            Some(1) => return Type::I64, // `x * 1` is `x`.
                            Some(scale) => {
                                self.line(&format!(
                                    "lea {}, %rax",
                                    Self::lea_operand("%rax", scale, 0)
                                ));
                                return Type::I64;
                            }
                            None => {}
                        }
                    }
                    self.line(&format!("imul ${k}, %rax, %rax"));
                    self.check_overflow();
                }
                BitAnd => self.line(&format!("and ${k}, %rax")),
                BitOr => self.line(&format!("or ${k}, %rax")),
                BitXor => self.line(&format!("xor ${k}, %rax")),
                // The shift count is masked to 6 bits by hardware, exactly as
                // Rust's `<<`/`>>` on `i64`; `sal`/`sar` take the count directly.
                // A count outside 0..=63 is a compile-time error when the shift
                // is constant (rejected in sema), so this arm only sees 0..=63.
                Shl => self.line(&format!("sal ${}, %rax", k & 63)),
                Shr => self.line(&format!("sar ${}, %rax", k & 63)),
                _ => unreachable!(),
            }
            return Type::I64;
        }

        // For these six ops the right operand can fold straight into the
        // instruction, so `x + y` skips the `mov y, %rcx` bounce. Only a
        // *register* operand folds: a memory operand measured slower on the
        // cores this was tuned on (the load lands in the instruction's critical
        // path), which is why the immediate-only rule above exists too.
        // `gen_expr(a)` leaves `%rax`/`%rcx`/`%rdx` alone, so a register operand
        // read from the promoted set is still live across it. The shifts are
        // excluded because the count must be in `%cl`.
        if matches!(op, Add | Sub | Mul | BitAnd | BitOr | BitXor) {
            if let Some(rb) = self.simple_operand(b) {
                if rb.starts_with('%') {
                    self.gen_expr(a);
                    match op {
                        Add => {
                            self.line(&format!("add {rb}, %rax"));
                            self.check_overflow();
                        }
                        Sub => {
                            self.line(&format!("sub {rb}, %rax"));
                            self.check_overflow();
                        }
                        Mul => {
                            self.line(&format!("imul {rb}, %rax"));
                            self.check_overflow();
                        }
                        BitAnd => self.line(&format!("and {rb}, %rax")),
                        BitOr => self.line(&format!("or {rb}, %rax")),
                        _ => self.line(&format!("xor {rb}, %rax")),
                    }
                    return Type::I64;
                }
            }
        }

        self.eval_operands(a, b, matches!(op, Add | Mul | BitAnd | BitOr | BitXor));
        match op {
            Add => {
                self.line("add %rcx, %rax");
                self.check_overflow();
            }
            Sub => {
                self.line("sub %rcx, %rax");
                self.check_overflow();
            }
            Mul => {
                self.line("imul %rcx, %rax");
                self.check_overflow();
            }
            BitAnd => self.line("and %rcx, %rax"),
            BitOr => self.line("or %rcx, %rax"),
            BitXor => self.line("xor %rcx, %rax"),
            // The variable shift count must be in `%cl`; `eval_operands`
            // already leaves the right operand in `%rcx`.
            Shl => {
                self.check_shift_count("%rcx");
                self.line("sal %cl, %rax");
            }
            Shr => {
                self.check_shift_count("%rcx");
                self.line("sar %cl, %rax");
            }
            _ => unreachable!(),
        }
        Type::I64
    }

    /// True when `d` fits an x86-64 `lea` displacement (a signed 32-bit value).
    /// Each constant in a folded chain is representable on its own, but their
    /// sum need not be, and GNU `as` rejects the out-of-range instruction.
    fn disp_ok(d: i64) -> bool {
        (-2_147_483_648..=2_147_483_647).contains(&d)
    }

    /// The value of `e` when it is an integer literal, negated or not, that fits
    /// in an `imm32`.
    fn small_imm(e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => Some(*v),
            Expr::Un(UnOp::Neg, inner) => match inner.as_ref() {
                Expr::Int(v) => v
                    .checked_neg()
                    .filter(|n| (-2_147_483_648..=2_147_483_647).contains(n)),
                _ => None,
            },
            _ => None,
        }
    }

    /// Emit a variable `Div`/`Mod` with the dividend in `%rax` and the divisor
    /// in `rd` (whose 32-bit half is `rd32`), leaving the result in `%rax`.
    ///
    /// A 64-bit `idiv` is ~30–40 cycles; the 32-bit `div`/`idiv` is ~20 and
    /// compiles to the same microcode as the 64-bit form on the cores this was
    /// tuned on, so when both operands fit in a 32-bit value the narrower
    /// instruction is simply cheaper. The operands are compared unsigned
    /// against 2^32 (`cmp`/`jae`, not a signed `jl`) so the test is exact: a
    /// negative dividend or divisor falls through to the 64-bit path, and no
    /// signed comparison trick is needed. `%rdx` is free — `cqto` has not run
    /// yet and a 32-bit `div` zeroes its own `%rdx` — and the dividend and
    /// divisor are still in `%rax`/`rd` when the fast path is taken.
    fn emit_div(&mut self, op: BinOp, rd: &str, rd32: &str) {
        let slow = self.new_label();
        let done = self.new_label();
        let ok = self.new_label();
        // Rust traps on a zero divisor even without `overflow-checks`. A zero
        // divisor is < 2^32, so without this it would take the 32-bit path below
        // and fault in `div`; check it before the fast/slow split.
        self.line(&format!("test {rd}, {rd}"));
        self.line("jz __rmc_divide_by_zero");
        // Both operands fit in 32 bits iff their OR has no bit above 31 set, so
        // one `or` and a `shr $32` replace the two compares against 2^32 (and
        // the 10-byte `movabs` that materialised it, which otherwise ran on
        // every iteration of a hot divide). The `or` is read unsigned: a
        // negative operand has a high bit set and takes the slow path, exactly
        // as the `jae` pair did. `%rdx` is scratch here (the slow path reloads
        // it, the fast path zeroes it below).
        self.line("mov %rax, %rdx");
        self.line(&format!("or {rd}, %rdx"));
        self.line("shr $32, %rdx");
        self.line(&format!("jnz {slow}"));
        // Both fit in 32 bits: the 32-bit divide is exact (the dividend's high
        // half is zero, so its `%rdx` contribution is zero) and cannot overflow.
        // The `shr $32, %rdx` above already left `%rdx` zero, which is the high
        // half the 32-bit `div` reads, so no `xor` is needed here.
        self.line(&format!("div {rd32}"));
        if matches!(op, BinOp::Mod) {
            self.line("mov %rdx, %rax");
        }
        self.line(&format!("jmp {done}"));
        // The 64-bit slow path is correct but rarely taken: both operands fit
        // in 32 bits on the overwhelming majority of divides. Left in the
        // fall-through it puts ~30 bytes of never-executed code between the
        // hot loop's `div` and its back edge; `mix`, whose `gcd` divides every
        // iteration, measures ~14% slower with it inline. Emit it as a cold
        // block after the function body instead. The `jmp {done}` above then
        // falls through to `{done}:`, so `trim_dead` drops it.
        self.cold_start();
        self.raw(&format!("{slow}:"));
        // `i64::MIN / -1` (and `% -1`) has no representable quotient and the
        // 64-bit `idiv` would fault; trap like Rust. Only a 64-bit-path dividend
        // can be `MIN` (a 32-bit one has the high bit clear), so the check sits
        // in the slow path and the fast path pays nothing for it. `%rdx` is
        // scratch here: `cqto` below overwrites it before it is read.
        self.line("movabs $0x8000000000000000, %rdx");
        self.line("cmp %rdx, %rax");
        self.line(&format!("jne {ok}"));
        self.line(&format!("cmp $-1, {rd}"));
        self.line("je __rmc_divide_overflow");
        self.raw(&format!("{ok}:"));
        self.line("cqto");
        self.line(&format!("idiv {rd}"));
        if matches!(op, BinOp::Mod) {
            self.line("mov %rdx, %rax");
        }
        // The cold block sits after the body, so it must jump back to the join
        // point; the hot path reaches `{done}` by fall-through (its own `jmp` is
        // dropped by `trim_dead`).
        self.line(&format!("jmp {done}"));
        self.cold_end();
        self.raw(&format!("{done}:"));
    }

    /// Division/modulo by a constant becomes a multiply, an arithmetic
    /// shift instead of `idiv`. Returns `false` for a variable divisor, a zero
    /// divisor, or a non-`i64` literal, so the caller falls back to `idiv`.
    ///
    /// `D` stays in `%rax` until the quotient or remainder is known; `%rdx` and
    /// `%r8` are scratch.
    fn gen_div_const(&mut self, op: BinOp, a: &Expr, b: &Expr) -> bool {
        let d = match Self::small_imm(b) {
            Some(v) if v != 0 => v,
            _ => return false,
        };
        self.gen_expr(a);
        self.line("mov %rax, %r8");
        if d == 1 {
            // The dividend is already in %rax.
        } else if d == -1 {
            // `i64::MIN / -1` and `i64::MIN % -1` overflow; `neg` sets OF for
            // exactly that operand, so the check is free. Rust traps here even
            // without `overflow-checks`, because `idiv` cannot produce the
            // wrapping result.
            self.line("neg %rax");
            self.line("jo __rmc_divide_overflow");
        } else {
            let mag = d.unsigned_abs();
            if mag.is_power_of_two() {
                // `d` is ±2^k. Arithmetic shift rounds toward -inf, so a
                // negative dividend needs a bias of 2^k-1 to round toward zero:
                // `%rdx` is 0 or -1 from the sign, then masked to 0 or 2^k-1.
                let k = mag.trailing_zeros();
                self.line("mov %rax, %rdx");
                self.line("sar $63, %rdx");
                self.line(&format!("shr ${}, %rdx", 64 - k));
                self.line("add %rdx, %rax");
                self.line(&format!("sar ${k}, %rax"));
                if d < 0 {
                    self.line("neg %rax");
                }
            } else {
                let (m, s, add) = Self::sdiv_magic(mag);
                self.mov_imm64(m, "%rcx");
                self.line("imul %rcx");
                if add {
                    // `imul` clobbers %rax with the low product, so the
                    // dividend comes from the %r8 copy, not from %rax.
                    self.line("add %r8, %rdx");
                }
                if s > 0 {
                    self.line(&format!("sar ${s}, %rdx"));
                }
                self.line("mov %r8, %rcx");
                self.line("shr $63, %rcx");
                self.line("add %rcx, %rdx");
                if d < 0 {
                    self.line("neg %rdx");
                }
                // For `%` the quotient is already in `%rdx`; leave it there and
                // take the remainder now, skipping the `%rdx`->`%rax`->`%rdx`
                // round-trip the shared tail below would emit.
                if matches!(op, BinOp::Mod) {
                    self.mov_imm64(d, "%rcx");
                    self.line("imul %rcx, %rdx");
                    self.line("sub %rdx, %r8");
                    self.line("mov %r8, %rax");
                    return true;
                }
                self.line("mov %rdx, %rax");
            }
        }
        if matches!(op, BinOp::Mod) {
            self.line("mov %rax, %rdx");
            self.mov_imm64(d, "%rcx");
            self.line("imul %rcx, %rdx");
            self.line("sub %rdx, %r8");
            self.line("mov %r8, %rax");
        }
        true
    }

    /// Magic multiplier and shift for signed division by `d > 1`, from Hacker's
    /// Delight via LLVM's `SignedDivisionByConstantInfo`. For a dividend `x`:
    ///
    /// ```text
    /// t = high64(signed(m) * x);  if add { t += x }  t >>= s
    /// q = t + (x >> 63)
    /// ```
    ///
    /// `add` is needed for roughly a quarter of divisors.
    /// Load a 64-bit constant into `reg`. `movabs` is only needed when the
    /// value does not fit in a sign-extended 32-bit immediate; otherwise the
    /// shorter `mov $k, reg` encodes the same value in 7 bytes instead of 10.
    /// The magic multiplier and the divisor are frequently small (a benchmark
    /// dividing by a small constant), so this trims the divide sequence and the
    /// artifact with it.
    fn mov_imm64(&mut self, v: i64, reg: &str) {
        if (-2_147_483_648..=2_147_483_647).contains(&v) {
            self.line(&format!("mov ${v}, {reg}"));
        } else {
            self.line(&format!("movabs ${v}, {reg}"));
        }
    }

    fn sdiv_magic(d: u64) -> (i64, u32, bool) {
        const N: u32 = 64;
        let smin: u128 = 1u128 << (N - 1);
        let ad = d as u128;
        // NC is the largest dividend whose remainder modulo `d` is `d - 1`.
        let anc = smin - 1 - (smin % ad);
        let (mut q1, mut r1) = (smin / anc, smin % anc);
        let (mut q2, mut r2) = (smin / ad, smin % ad);
        let mut p = N - 1;
        loop {
            p += 1;
            q1 <<= 1;
            r1 <<= 1;
            if r1 >= anc {
                q1 += 1;
                r1 -= anc;
            }
            q2 <<= 1;
            r2 <<= 1;
            if r2 >= ad {
                q2 += 1;
                r2 -= ad;
            }
            let delta = ad - r2;
            if !(q1 < delta || (q1 == delta && r1 == 0)) {
                break;
            }
        }
        let m = q2 + 1;
        let add = m > i64::MAX as u128;
        (m as i64, p - N, add)
    }

    /// An operand `disp(%rbp,%rax,8)` for `base[idx]` where `base` is a local
    /// array of 8-byte elements. The caller must have the index value in `%rax`.
    /// Returns `None` for anything else (a wider element, a non-local base, an
    /// index that is neither a constant nor a plain variable), so the caller
    /// falls back to computing the address with the element stride applied.
    fn array_elem_operand(&self, base: &Expr, idx: &Expr) -> Option<String> {
        let (slot, n) = match base {
            Expr::Var { slot, ty: Type::Array(_, n), .. } => (*slot, *n),
            _ => return None,
        };
        // A wide element (a nested array or a struct) is addressed by the
        // general path, which scales the index by the element size.
        if Self::elem_stride(base) != 1 {
            return None;
        }
        match idx {
            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => {
                // Constant index: no register needed at all.
                Some(format!("{}(%rbp)", self.array_off(slot, n) + *v as i32 * 8))
            }
            Expr::Var { slot: _, ty: Type::I64, .. } => {
                // The index lives in a slot; the caller must load it into %rax
                // and bake the array base offset into the displacement.
                Some(format!("{}(%rbp,%rax,8)", self.array_off(slot, n)))
            }
            _ => None,
        }
    }

    /// Emit a `cmp` that leaves the flags for `a - b`, i.e. the flags for
    /// `a op b`. Returns false when neither operand can be named without a
    /// register, so the caller can fall back to the general path. x86 rejects
    /// memory-vs-memory and immediate-vs-immediate, and needs an explicit `q`
    /// suffix when an immediate is compared against memory.
    fn emit_cmp(&mut self, a: &Expr, b: &Expr) -> bool {
        let av = self.simple_operand(a);
        let bv = self.simple_operand(b);
        match (av, bv) {
            (Some(x), Some(y)) => {
                match (x.starts_with('$'), y.starts_with('$')) {
                    // Two immediates: materialise one so they can be compared.
                    (true, true) => {
                        self.line(&format!("mov {x}, %rax"));
                        self.line(&format!("cmp {y}, %rax"));
                    }
                    // `a` is an immediate and `b` in memory. x86 cannot compare
                    // an immediate against memory with the immediate first, so
                    // load `a` into `%rcx` and compare `b` against it, leaving
                    // the flags of `a - b`.
                    (true, false) => {
                        self.line(&format!("mov {x}, %rcx"));
                        self.line(&format!("cmp {y}, %rcx"));
                    }
                    // `a` in memory, `b` immediate: `cmpq $b, a` is `a - b`.
                    (false, true) => self.line(&format!("cmpq {y}, {x}")),
                    // Two register-free operands. `cmp y, x` is legal unless
                    // both are memory (x86 has no mem,mem compare); only then
                    // must `b` be materialised into a register.
                    (false, false) => {
                        let x_mem = !x.starts_with('%') && !x.starts_with('$');
                        let y_mem = !y.starts_with('%') && !y.starts_with('$');
                        if x_mem && y_mem {
                            self.gen_expr(b);
                            self.line(&format!("cmp %rax, {x}"));
                        } else {
                            self.line(&format!("cmp {y}, {x}"));
                        }
                    }
                }
                true
            }
            (Some(x), None) => {
                self.gen_expr(b);
                if x.starts_with('$') {
                    self.line(&format!("mov {x}, %rcx"));
                    self.line("cmp %rax, %rcx");
                } else {
                    self.line(&format!("cmp %rax, {x}"));
                }
                true
            }
            (None, Some(y)) => {
                self.gen_expr(a);
                self.line(&format!("cmp {y}, %rax"));
                true
            }
            (None, None) => false,
        }
    }

    /// Evaluate `a` into `%rax` and `b` into `%rcx`, the operand registers the
    /// arithmetic and comparison paths read. `commutative` lets the two-operand
    /// cases skip a register bounce: when the operation does not care about
    /// order, the spilled operand is placed in `%rcx` directly instead of being
    /// routed through `%rax`, collapsing two moves into one. Callers must pass
    /// `false` for anything order-sensitive — comparisons and `sub`/`div`/`mod`.
    fn eval_operands(&mut self, a: &Expr, b: &Expr, commutative: bool) {
        let ra = self.simple_operand(a);
        let rb = self.simple_operand(b);
        match (ra, rb) {
            (Some(la), Some(rb)) => {
                self.line(&format!("mov {la}, %rax"));
                self.line(&format!("mov {rb}, %rcx"));
            }
            (None, Some(rb)) => {
                self.gen_expr(a);
                self.line(&format!("mov {rb}, %rcx"));
            }
            (Some(la), None) => {
                self.gen_expr(b);
                if commutative {
                    // `%rax` already holds `b`; the operation reads both
                    // registers, so `la` only needs to reach `%rcx`.
                    self.line(&format!("mov {la}, %rcx"));
                } else {
                    self.line("mov %rax, %rcx");
                    self.line(&format!("mov {la}, %rax"));
                }
            }
            (None, None) => {
                self.gen_expr(a);
                self.push_rax();
                self.gen_expr(b);
                if commutative {
                    self.pop_to("%rcx");
                } else {
                    self.line("mov %rax, %rcx");
                    self.pop_to("%rax");
                }
            }
        }
    }

    /// Branch to `target` when `cond` is false. Comparisons are emitted as a
    /// direct `cmp`/`jcc` pair, which keeps loop and `if` conditions tight.
    fn gen_branch_false(&mut self, cond: &Expr, target: &str) {
        if let Expr::Bin(op, a, b) = cond {
            if let Some(cc) = invert_cc(*op) {
                if !self.emit_cmp(a, b) {
                    self.eval_operands(a, b, false);
                    self.line("cmp %rcx, %rax");
                }
                self.line(&format!("{cc} {target}"));
                return;
            }
        }
        // `!(a < b)` jumps when the comparison holds.
        if let Expr::Un(UnOp::Not, inner) = cond {
            if let Expr::Bin(op, a, b) = inner.as_ref() {
                if let Some(cc) = setcc_cc(*op) {
                    if !self.emit_cmp(a, b) {
                        self.eval_operands(a, b, false);
                        self.line("cmp %rcx, %rax");
                    }
                    self.line(&format!("{cc} {target}"));
                    return;
                }
            }
        }
        self.gen_expr(cond);
        self.line("test %rax, %rax");
        self.line(&format!("jz {target}"));
    }

    /// The dual of `gen_branch_false`: jump to `target` when `cond` holds,
    /// otherwise fall through. Used to close a rotated loop, where the test sits
    /// at the bottom and the back edge is the conditional jump.
    fn gen_branch_true(&mut self, cond: &Expr, target: &str) {
        if let Expr::Bin(op, a, b) = cond {
            if let Some(cc) = setcc_cc(*op) {
                if !self.emit_cmp(a, b) {
                    self.eval_operands(a, b, false);
                    self.line("cmp %rcx, %rax");
                }
                self.line(&format!("{cc} {target}"));
                return;
            }
        }
        // `!(a < b)` jumps back when the comparison is false.
        if let Expr::Un(UnOp::Not, inner) = cond {
            if let Expr::Bin(op, a, b) = inner.as_ref() {
                if let Some(cc) = invert_cc(*op) {
                    if !self.emit_cmp(a, b) {
                        self.eval_operands(a, b, false);
                        self.line("cmp %rcx, %rax");
                    }
                    self.line(&format!("{cc} {target}"));
                    return;
                }
            }
        }
        self.gen_expr(cond);
        self.line("test %rax, %rax");
        self.line(&format!("jnz {target}"));
    }

    fn gen_call(
        &mut self,
        name: &str,
        args: &[Expr],
        arg_slots: &[(i32, usize)],
        ret_slot: i32,
        ret_ty: &Type,
    ) -> Type {
        const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

        // Arguments are evaluated in source order. An aggregate is passed by
        // value: it is copied into its hidden frame slot, and that slot's
        // address is what travels in the argument register. Each argument is
        // pushed as it is evaluated (aggregate: its slot address; scalar: its
        // value), so the pushes are left-to-right and the register loads below
        // are a single reverse walk. Copying every aggregate only after all
        // scalars would run the arguments out of order.
        let is_agg: Vec<bool> = arg_slots.iter().map(|(s, _)| *s >= 0).collect();

        let direct = is_agg.iter().all(|a| !*a)
            && (args.iter().all(|a| self.arg_is_simple(a))
                || (args.len() <= 2
                    && !args.iter().any(Self::contains_call)
                    && !args.iter().any(Self::clobbers_arg_regs)));
        if direct {
            for (a, r) in args.iter().zip(ARG_REGS.iter()) {
                self.gen_expr_into_reg(a, r);
            }
        } else {
            for ((a, agg), (slot, n)) in args.iter().zip(is_agg.iter()).zip(arg_slots.iter()) {
                if *agg {
                    let base = self.array_off(*slot, *n);
                    self.gen_expr(a); // the aggregate's address is in %rax
                    self.line("mov %rax, %rsi");
                    self.line(&format!("lea {base}(%rbp), %rdi"));
                    self.copy_words_rdi_rsi(*n);
                    self.line(&format!("lea {base}(%rbp), %rax"));
                } else {
                    self.gen_expr(a);
                }
                self.push_rax();
            }
            // Pushes were left-to-right, so pop right-to-left into the ABI
            // registers, one per argument.
            for i in (0..args.len()).rev() {
                self.pop_to(ARG_REGS[i]);
            }
        }

        // The return buffer pointer goes in %r10, which is caller-saved and not
        // one of the argument registers, so it survives argument setup.
        if ret_slot >= 0 {
            let n = ret_ty.size() / 8;
            self.line(&format!("lea {}(%rbp), %r10", self.array_off(ret_slot, n)));
        }
        // Spills live in frame slots, so `%rsp` is already 16-byte aligned at
        // every call and needs no pad here.
        self.line(&format!("call {name}"));
        // An aggregate result's address (the hidden buffer) is in %rax; a
        // scalar result is already in %rax.
        ret_ty.clone()
    }

    /// An argument that loads into a register without using the `%rcx`/`%rdx`
    /// scratch registers that later arguments would clobber.
    fn arg_is_simple(&self, e: &Expr) -> bool {
        self.simple_operand(e).is_some() || matches!(e, Expr::Str(_))
    }

    /// True when evaluating `e` may perform a call.
    fn contains_call(e: &Expr) -> bool {
        match e {
            Expr::Call { .. } => true,
            // A struct or array literal is materialised through a copy that
            // uses scratch registers, so the direct-argument shortcut (which
            // requires register preservation) must not select it.
            Expr::StructLit { .. } | Expr::ArrayLit(..) => true,
            Expr::Bin(_, a, b) => Self::contains_call(a) || Self::contains_call(b),
            Expr::Un(_, a) | Expr::Cast(a, _) => Self::contains_call(a),
            Expr::Index(a, b) => Self::contains_call(a) || Self::contains_call(b),
            Expr::Field { base, .. } => Self::contains_call(base),
            // Every expression that evaluates a child can call, so each must be
            // covered: a missed variant lets the direct-argument shortcut pick
            // an argument whose nested call clobbers an earlier argument
            // register, and the assignment reorder must also see a call inside
            // a place.
            Expr::EnumCtor { args, .. } => args.iter().any(Self::contains_call),
            Expr::VariantTest { scrutinee, .. } | Expr::BoolTest { scrutinee, .. } => {
                Self::contains_call(scrutinee)
            }
            Expr::EnumPayload { recv, .. } => Self::contains_call(recv),
            Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => Self::contains_call(expr),
            Expr::ArrayList(v) => v.iter().any(Self::contains_call),
            Expr::If(c, t, els) => {
                Self::contains_call(c)
                    || Self::block_has_call(t)
                    || els.as_ref().map_or(false, Self::block_has_call)
            }
            Expr::Block(b) | Expr::Loop(b, _, _, _) => Self::block_has_call(b),
            Expr::MatchBoundary(b, _) => Self::block_has_call(b),
            _ => false,
        }
    }

    /// True when forming the address of `e` can change observable state: a
    /// call, or a block/if that assigns a local. The value-before-place reorder
    /// must key on this, not `contains_call`: `a[{ x = 2; 0 }] = x` computes the
    /// place (a store) with no call, and reading the place first would store the
    /// new `x` instead of the value `x` held before the place ran.
    fn place_has_effect(e: &Expr) -> bool {
        match e {
            Expr::Block(b) | Expr::MatchBoundary(b, _) => Self::block_has_effect(b),
            Expr::If(c, t, els) => {
                Self::place_has_effect(c)
                    || Self::block_has_effect(t)
                    || els.as_ref().map_or(false, |b| Self::block_has_effect(b))
            }
            Expr::Index(a, b) => Self::place_has_effect(a) || Self::place_has_effect(b),
            Expr::Field { base, .. } => Self::place_has_effect(base),
            Expr::Deref { expr, .. } | Expr::AddrOf { expr, .. } => Self::place_has_effect(expr),
            Expr::EnumPayload { recv, .. } => Self::place_has_effect(recv),
            // An arithmetic, unary or cast sub-expression can hide a block that
            // assigns a local (`r[{ y = 2; 0 } + 0] = y`), so the operands are
            // traversed rather than reduced to a call check.
            Expr::Bin(_, a, b) => Self::place_has_effect(a) || Self::place_has_effect(b),
            Expr::Un(_, a) | Expr::Cast(a, _) => Self::place_has_effect(a),
            Expr::ArrayLit(v, _) => Self::place_has_effect(v),
            Expr::ArrayList(v) => v.iter().any(Self::place_has_effect),
            // A call is an effect by itself; its arguments cannot make it worse.
            Expr::Call { .. } | Expr::MethodCall { .. } => true,
            Expr::StructLit { fields, .. } => fields.iter().any(|f| Self::place_has_effect(&f.value)),
            Expr::EnumCtor { args, .. } => args.iter().any(Self::place_has_effect),
            Expr::VariantTest { scrutinee, .. } | Expr::BoolTest { scrutinee, .. } => {
                Self::place_has_effect(scrutinee)
            }
            _ => Self::contains_call(e),
        }
    }

    fn block_has_effect(b: &Block) -> bool {
        b.stmts.iter().any(Self::stmt_has_effect)
            || b.tail.as_deref().map_or(false, Self::place_has_effect)
    }

    fn stmt_has_effect(s: &Stmt) -> bool {
        match s {
            Stmt::Let { init, .. } => init.as_ref().map_or(false, Self::place_has_effect),
            // An assignment writes a local, which a later read can observe.
            Stmt::Assign { .. } => true,
            Stmt::Expr(e) => Self::place_has_effect(e),
            Stmt::Return(v) => v.as_ref().map_or(false, Self::place_has_effect),
            Stmt::For { start, end, body, .. } => {
                Self::place_has_effect(start)
                    || Self::place_has_effect(end)
                    || Self::block_has_effect(body)
            }
            Stmt::While(..) => true,
            Stmt::Break(v, _) => v.as_ref().map_or(false, Self::place_has_effect),
            Stmt::Continue(_) => false,
        }
    }

    fn block_has_call(b: &Block) -> bool {
        b.stmts.iter().any(Self::stmt_has_call)
            || b.tail.as_deref().map_or(false, Self::contains_call)
    }

    /// True when evaluating `e` can clobber `%rdi`/`%rsi` without
    /// `contains_call` noticing. Two cases: copying a whole aggregate into a
    /// local, which runs through `%rdi`/`%rsi` (`copy_words_rdi_rsi`), and
    /// `&<aggregate>` with a materialised temporary slot, which copies the
    /// aggregate into that slot the same way. The walk recurses through every
    /// expression form, because either can hide inside an operand (`&{ s }`,
    /// `g(x, &{ s })`) with no call in sight, and the direct-argument shortcut
    /// must not select such an argument.
    fn clobbers_arg_regs(e: &Expr) -> bool {
        match e {
            Expr::Block(b) | Expr::Loop(b, _, _, _) => Self::block_clobbers(b),
            Expr::MatchBoundary(b, _) => Self::block_clobbers(b),
            Expr::If(_, t, els) => {
                Self::block_clobbers(t)
                    || els.as_ref().map_or(false, |b| Self::block_clobbers(b))
            }
            Expr::AddrOf { expr, ty, slot, .. } => {
                (*slot >= 0 && ty.is_aggregate()) || Self::clobbers_arg_regs(expr)
            }
            Expr::Bin(_, a, b) => Self::clobbers_arg_regs(a) || Self::clobbers_arg_regs(b),
            Expr::Un(_, a) | Expr::Cast(a, _) => Self::clobbers_arg_regs(a),
            Expr::Index(a, b) => Self::clobbers_arg_regs(a) || Self::clobbers_arg_regs(b),
            Expr::Field { base, .. } => Self::clobbers_arg_regs(base),
            Expr::Deref { expr, .. } => Self::clobbers_arg_regs(expr),
            Expr::Call { args, .. } => args.iter().any(Self::clobbers_arg_regs),
            Expr::MethodCall { recv, args, .. } => {
                Self::clobbers_arg_regs(recv) || args.iter().any(Self::clobbers_arg_regs)
            }
            Expr::EnumCtor { args, .. } => args.iter().any(Self::clobbers_arg_regs),
            Expr::StructLit { fields, .. } => {
                fields.iter().any(|f| Self::clobbers_arg_regs(&f.value))
            }
            Expr::ArrayLit(v, _) => Self::clobbers_arg_regs(v),
            Expr::ArrayList(v) => v.iter().any(Self::clobbers_arg_regs),
            Expr::VariantTest { scrutinee, .. } | Expr::BoolTest { scrutinee, .. } => {
                Self::clobbers_arg_regs(scrutinee)
            }
            Expr::EnumPayload { recv, .. } => Self::clobbers_arg_regs(recv),
            _ => false,
        }
    }

    fn block_clobbers(b: &Block) -> bool {
        b.stmts.iter().any(Self::stmt_clobbers)
            || b.tail.as_deref().map_or(false, Self::clobbers_arg_regs)
    }

    fn stmt_clobbers(s: &Stmt) -> bool {
        match s {
            Stmt::Let { init: Some(e), .. } => Self::value_copies_aggregate(e),
            // An aggregate store copies word by word through `%rdi`/`%rsi`
            // (`copy_words_rdi_rsi`), whether the destination is a whole local,
            // a struct field or an array element, so all three clobber the
            // argument registers.
            Stmt::Assign { target, .. } => match target {
                Expr::Var { ty, .. } | Expr::Field { ty, .. } => ty.is_aggregate(),
                Expr::Index(base, _) => matches!(
                    base.as_ref(),
                    Expr::Var { ty: Type::Array(e, _), .. } if e.is_aggregate()
                ),
                _ => false,
            },
            _ => false,
        }
    }

    /// True when `e` produces an aggregate by copying, which goes through
    /// `%rdi`/`%rsi`. A literal is excluded because `contains_call` already
    /// rejects it; a call result is excluded because it is a call.
    fn value_copies_aggregate(e: &Expr) -> bool {
        match e {
            Expr::Var { ty, .. } | Expr::Field { ty, .. } => ty.is_aggregate(),
            Expr::Block(_) | Expr::MatchBoundary(_, _) | Expr::Loop(..) | Expr::If(..) => {
                Self::clobbers_arg_regs(e)
            }
            _ => false,
        }
    }

    fn stmt_has_call(s: &Stmt) -> bool {
        match s {
            Stmt::Let { init, .. } => init.as_ref().map_or(false, Self::contains_call),
            Stmt::Assign { target, value } => {
                Self::contains_call(target) || Self::contains_call(value)
            }
            Stmt::Expr(e) => Self::contains_call(e),
            Stmt::Return(v) => v.as_ref().map_or(false, Self::contains_call),
            Stmt::For { start, end, body, .. } => {
                Self::contains_call(start) || Self::contains_call(end) || Self::block_has_call(body)
            }
            // A loop body might call on some iteration; be conservative.
            Stmt::While(..) => true,
            Stmt::Break(v, _) => v.as_ref().map_or(false, Self::contains_call),
            Stmt::Continue(_) => false,
        }
    }

}

/// The `setcc` mnemonic for a comparison operator.
fn setcc(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "sete",
        BinOp::Ne => "setne",
        BinOp::Lt => "setl",
        BinOp::Le => "setle",
        BinOp::Gt => "setg",
        BinOp::Ge => "setge",
        _ => unreachable!(),
    }
}

/// The 8-bit sub-register of a 64-bit register name (`%rax` -> `%al`). The
/// high-byte registers (`%ah`..) are never used, so the low-byte encoding is
/// always the plain one.
fn reg8(r: &str) -> &'static str {
    match r {
        "%rax" => "%al",
        "%rbx" => "%bl",
        "%rcx" => "%cl",
        "%rdx" => "%dl",
        "%rsi" => "%sil",
        "%rdi" => "%dil",
        "%r8" => "%r8b",
        "%r9" => "%r9b",
        "%r10" => "%r10b",
        "%r11" => "%r11b",
        "%r12" => "%r12b",
        "%r13" => "%r13b",
        "%r14" => "%r14b",
        "%r15" => "%r15b",
        _ => unreachable!("no 8-bit form of {r}"),
    }
}

/// The 16-bit sub-register of a 64-bit register name (`%rax` -> `%ax`).
fn reg16(r: &str) -> &'static str {
    match r {
        "%rax" => "%ax",
        "%rbx" => "%bx",
        "%rcx" => "%cx",
        "%rdx" => "%dx",
        "%rsi" => "%si",
        "%rdi" => "%di",
        "%r8" => "%r8w",
        "%r9" => "%r9w",
        "%r10" => "%r10w",
        "%r11" => "%r11w",
        "%r12" => "%r12w",
        "%r13" => "%r13w",
        "%r14" => "%r14w",
        "%r15" => "%r15w",
        _ => unreachable!("no 16-bit form of {r}"),
    }
}

/// The 32-bit half of a 64-bit register mnemonic, for the `div`/`idiv` fast
/// path in [`Codegen::emit_div`].
fn r32(r: &str) -> &'static str {
    match r {
        "%rax" => "%eax",
        "%rbx" => "%ebx",
        "%rcx" => "%ecx",
        "%rdx" => "%edx",
        "%rsi" => "%esi",
        "%rdi" => "%edi",
        "%r8" => "%r8d",
        "%r9" => "%r9d",
        "%r10" => "%r10d",
        "%r11" => "%r11d",
        "%r12" => "%r12d",
        "%r13" => "%r13d",
        "%r14" => "%r14d",
        "%r15" => "%r15d",
        _ => unreachable!("no 32-bit form of {r}"),
    }
}

/// The `jcc` mnemonic that jumps when a comparison is *false*, for `a op b`.
fn invert_cc(op: BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Eq => "jne",
        BinOp::Ne => "je",
        BinOp::Lt => "jge",
        BinOp::Le => "jg",
        BinOp::Gt => "jle",
        BinOp::Ge => "jl",
        _ => return None,
    })
}

/// The `jcc` mnemonic that jumps when a comparison holds.
fn setcc_cc(op: BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::Eq => "je",
        BinOp::Ne => "jne",
        BinOp::Lt => "jl",
        BinOp::Le => "jle",
        BinOp::Gt => "jg",
        BinOp::Ge => "jge",
        _ => return None,
    })
}

