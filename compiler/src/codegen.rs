//! Direct x86-64 (System V, AT&T syntax) code generator.
//!
//! Expression results live in `%rax` and temporaries are pushed on the machine
//! stack, so the generator needs no register allocator. Locals are different:
//! the hottest few scalars in each function are promoted into callee-saved
//! registers for the whole function (see `promote`), and the rest occupy an
//! 8-byte slot at `-(slot + 1 + nregs)*8(%rbp)`, below the saved registers and
//! growing downward from `%rbp`.

use crate::ast::*;
use std::fmt::Write as _;

pub struct Codegen {
    out: String,
    label: u32,
    /// Number of 8-byte pushes currently live, used for 16-byte stack alignment.
    sp_off: i32,
    str_id: u32,
    strings: Vec<(String, String)>, // (label, bytes)
    loops: Vec<(String, String)>,   // (continue_label, break_label)
    epilogue: String,
    /// Slot -> callee-saved register for locals promoted out of memory in the
    /// function currently being generated. Empty means every local is in memory.
    regs: Vec<(i32, &'static str)>,
    /// When set, every `+`, `-` and `*` is followed by a jump-if-overflow to the
    /// runtime trap. Off by default because it costs a branch per operation.
    overflow: bool,
    /// Frame slot holding the hidden return-buffer pointer for the function
    /// being generated (`-1` when the return type is scalar or unit).
    ret_slot: i32,
    /// Return type of the function being generated, needed to route `return`.
    ret_ty: Type,
}

/// Callee-saved registers eligible for promoting a local out of memory. They
/// survive calls, so a promoted variable does not need to be spilled around
/// one. `%rbp` and `%rsp` are excluded; `%rbx` is listed first because a short
/// register name keeps the generated assembly readable.
const ALLOC_REGS: [&str; 5] = ["%rbx", "%r12", "%r13", "%r14", "%r15"];

impl Codegen {
    pub fn new() -> Self {
        Codegen {
            out: String::with_capacity(1 << 16),
            label: 0,
            sp_off: 0,
            str_id: 0,
            strings: Vec::new(),
            loops: Vec::new(),
            epilogue: String::new(),
            regs: Vec::new(),
            overflow: false,
            ret_slot: -1,
            ret_ty: Type::Unit,
        }
    }

    /// Turn on overflow checks for `+`, `-` and `*`.
    pub fn overflow_checks(mut self) -> Self {
        self.overflow = true;
        self
    }

    /// The register holding local `slot`, if it was promoted out of memory.
    fn reg_of(&self, slot: i32) -> Option<&'static str> {
        self.regs.iter().find(|(s, _)| *s == slot).map(|(_, r)| *r)
    }

    fn line(&mut self, s: &str) {
        self.out.push_str("\t");
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn raw(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    /// Number of callee-saved registers currently holding promoted locals.
    fn nregs(&self) -> i32 {
        self.regs.len() as i32
    }

    /// Byte offset of stack `slot` relative to `%rbp`. Slots grow downward, and
    /// the saved promoted registers sit in the lowest bytes of the frame, so
    /// every slot is shifted past them and never overlaps `%rbp` or a saved
    /// register.
    fn off(&self, slot: i32) -> i32 {
        -(slot + 1 + self.nregs()) * 8
    }

    /// Address of element 0 of an `n`-element array occupying `slot..slot+n`.
    /// Slots are laid out downward, so the lowest address is that of the last
    /// slot; elements are then addressed upward from there as `base + i*8`.
    fn array_off(&self, slot: i32, n: usize) -> i32 {
        -(slot + n as i32 + self.nregs()) * 8
    }

    fn new_label(&mut self) -> String {
        let l = format!(".L{}", self.label);
        self.label += 1;
        l
    }

    fn push_rax(&mut self) {
        self.line("push %rax");
        self.sp_off += 1;
    }

    fn pop_to(&mut self, reg: &str) {
        self.line(&format!("pop {reg}"));
        self.sp_off -= 1;
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
        self.sp_off = 0;
        self.epilogue = self.new_label();
        self.ret_slot = f.ret_slot;
        self.ret_ty = f.ret.clone();
        // Promote the most-used scalar locals into callee-saved registers.
        self.regs = Self::promote(f);
        let saved: Vec<&'static str> = self.regs.iter().map(|(_, r)| *r).collect();

        let nregs = self.regs.len();
        self.raw(&format!(".globl {}", f.name));
        self.raw(&format!(".type {}, @function", f.name));
        self.raw(&format!("{}:", f.name));
        self.line("push %rbp");
        self.line("mov %rsp, %rbp");
        // Save the registers used for promoted locals. The frame is padded to a
        // multiple of 16 so that, counting these pushes, %rsp stays 16-byte
        // aligned at every call.
        for r in &saved {
            self.line(&format!("push {r}"));
        }
        let saved_bytes = nregs as i32 * 8;
        let slots_bytes = f.nslots * 8;
        // The total depth below %rbp (saved registers plus frame) must be a
        // multiple of 16 so %rsp is 16-byte aligned at every call.
        let frame = ((slots_bytes + saved_bytes + 15) / 16) * 16 - saved_bytes;
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
        // The saved registers sit just below %rbp; point %rsp at the lowest one
        // and pop them back, which also discards the frame.
        if nregs > 0 {
            self.line(&format!("lea -{}(%rbp), %rsp", nregs * 8));
            for r in saved.iter().rev() {
                self.line(&format!("pop {r}"));
            }
        }
        self.line("leave");
        self.line("ret");
        self.raw(&format!(".size {}, .-{}", f.name, f.name));
        self.regs.clear();
    }

    /// Copy `n` 8-byte words from the address in `%rsi` to the address in `%rdi`.
    /// `%rax` and, for larger counts, `%rcx` are clobbered.
    fn copy_words_rdi_rsi(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        if n <= 8 {
            for i in 0..n {
                self.line(&format!("mov {}(%rsi), %rax", i as i32 * 8));
                self.line(&format!("mov %rax, {}(%rdi)", i as i32 * 8));
            }
        } else {
            let l = self.new_label();
            self.line(&format!("mov ${n}, %rcx"));
            self.raw(&format!("{l}:"));
            self.line("mov (%rsi), %rax");
            self.line("mov %rax, (%rdi)");
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

    /// Choose up to `ALLOC_REGS.len()` scalar locals to keep in registers for
    /// the whole function, ordered by how often they are mentioned. Only slots
    /// that are never the base of an array access qualify, since an array needs
    /// a memory address. Ties break toward the lowest slot, which is where
    /// parameters live.
    fn promote(f: &FnDef) -> Vec<(i32, &'static str)> {
        let mut counts: Vec<(i32, u32)> = (0..f.nslots).map(|s| (s, 0u32)).collect();
        let mut array_slots: Vec<i32> = Vec::new();
        Self::count_block(&f.body, &mut counts, &mut array_slots);
        // `counts` is built in slot order and not reordered until after this
        // loop, so slot `s` lives at index `s`. Zeroing by index keeps this
        // linear even for an array with hundreds of thousands of elements.
        for s in &array_slots {
            counts[*s as usize].1 = 0;
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        counts
            .into_iter()
            .filter(|(_, c)| *c > 0)
            .take(ALLOC_REGS.len())
            .zip(ALLOC_REGS.iter())
            .map(|((slot, _), reg)| (slot, *reg))
            .collect()
    }

    fn count_block(b: &Block, counts: &mut Vec<(i32, u32)>, array_slots: &mut Vec<i32>) {
        for s in &b.stmts {
            Self::count_stmt(s, counts, array_slots);
        }
        if let Some(t) = &b.tail {
            Self::count_expr(t, counts, array_slots);
        }
    }

    fn count_stmt(s: &Stmt, counts: &mut Vec<(i32, u32)>, array_slots: &mut Vec<i32>) {
        match s {
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    Self::count_expr(e, counts, array_slots);
                }
            }
            Stmt::Assign { target, value } => {
                Self::count_expr(target, counts, array_slots);
                Self::count_expr(value, counts, array_slots);
            }
            Stmt::Expr(e) => Self::count_expr(e, counts, array_slots),
            Stmt::While(c, b) => {
                Self::count_expr(c, counts, array_slots);
                Self::count_block(b, counts, array_slots);
            }
            Stmt::Loop(b) => Self::count_block(b, counts, array_slots),
            Stmt::For { start, end, body, .. } => {
                Self::count_expr(start, counts, array_slots);
                Self::count_expr(end, counts, array_slots);
                Self::count_block(body, counts, array_slots);
            }
            Stmt::Return(v) => {
                if let Some(e) = v {
                    Self::count_expr(e, counts, array_slots);
                }
            }
            Stmt::Break | Stmt::Continue => {}
        }
    }

    fn count_expr(e: &Expr, counts: &mut Vec<(i32, u32)>, array_slots: &mut Vec<i32>) {
        match e {
            Expr::Var { slot, ty, .. } => {
                counts[*slot as usize].1 += 1;
                // A promoted local is named by its register, so an aggregate
                // (which needs an address) must keep all of its slots in memory.
                if ty.is_aggregate() {
                    for k in 0..(ty.size() / 8) as i32 {
                        array_slots.push(*slot + k);
                    }
                }
            }
            Expr::Index(base, idx) => {
                Self::count_expr(base, counts, array_slots);
                Self::count_expr(idx, counts, array_slots);
            }
            Expr::Field { base, .. } => Self::count_expr(base, counts, array_slots),
            Expr::StructLit { fields, .. } => {
                for fi in fields {
                    Self::count_expr(&fi.value, counts, array_slots);
                }
            }
            Expr::Bin(_, a, b) => {
                Self::count_expr(a, counts, array_slots);
                Self::count_expr(b, counts, array_slots);
            }
            Expr::Un(_, a) | Expr::Cast(a, _) => Self::count_expr(a, counts, array_slots),
            Expr::Call { args, .. } => {
                for a in args {
                    Self::count_expr(a, counts, array_slots);
                }
            }
            Expr::If(c, t, els) => {
                Self::count_expr(c, counts, array_slots);
                Self::count_block(t, counts, array_slots);
                if let Some(e) = els {
                    Self::count_block(e, counts, array_slots);
                }
            }
            Expr::Block(b) => Self::count_block(b, counts, array_slots),
            Expr::ArrayLit(v, _) => Self::count_expr(v, counts, array_slots),
            Expr::Int(_) | Expr::Bool(_) | Expr::Str(_) => {}
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
                        // `[v; n]` splats one value across every element.
                        Some(Expr::ArrayLit(v, _)) => {
                            self.gen_expr(v);
                            self.fill_aggregate(*slot, elem.size(), *n);
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
                } else if let Type::Struct(_, size) = &vty {
                    match init {
                        Some(Expr::StructLit { fields, .. }) => {
                            let base = self.array_off(*slot, size / 8);
                            for fi in fields {
                                self.gen_expr(&fi.value);
                                let off = base + fi.offset;
                                self.line(&format!("mov %rax, {off}(%rbp)"));
                            }
                        }
                        // No initializer: the struct is zero-filled.
                        None => {
                            self.line("xor %eax, %eax");
                            self.fill_aggregate(*slot, 8, size / 8);
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
                    self.gen_expr(e);
                    self.store_slot(*slot);
                } else {
                    match self.reg_of(*slot) {
                        Some(r) => self.line(&format!("xor {r}, {r}")),
                        None => self.line(&format!("movq $0, {}(%rbp)", self.off(*slot))),
                    }
                }
            }
            Stmt::Assign { target, value } => {
                match target {
                    Expr::Var { slot, .. } => {
                        self.gen_expr(value);
                        self.store_slot(*slot);
                    }
                    Expr::Field { base, offset, .. } => {
                        match self.field_operand(base, *offset) {
                            Some(op) => {
                                self.gen_expr(value);
                                self.line(&format!("mov %rax, {op}"));
                            }
                            None => {
                                self.gen_addr(target);
                                self.push_rax();
                                self.gen_expr(value);
                                self.pop_to("%rcx");
                                self.line("mov %rax, (%rcx)");
                            }
                        }
                    }
                    Expr::Index(base, idx) => {
                        match self.array_elem_operand(base, idx) {
                            // Constant index: one store.
                            Some(op) if !op.contains("%rax") => {
                                self.gen_expr(value);
                                self.line(&format!("mov %rax, {op}"));
                            }
                            // Variable index: load the index into %rax, then
                            // store through `disp(%rbp,%rax,8)`.
                            Some(op) => {
                                match self.simple_operand(value) {
                                    Some(vs) if vs.starts_with('$') => {
                                        self.gen_expr(idx);
                                        self.line(&format!("movq {vs}, {op}"));
                                    }
                                    Some(vs) => {
                                        self.gen_expr(idx);
                                        self.line(&format!("mov {vs}, %rcx"));
                                        self.line(&format!("mov %rcx, {op}"));
                                    }
                                    None => {
                                        self.gen_expr(value);
                                        self.push_rax();
                                        self.gen_expr(idx);
                                        self.pop_to("%rcx");
                                        self.line(&format!("mov %rcx, {op}"));
                                    }
                                }
                            }
                            None => {
                                self.gen_addr(target);
                                self.push_rax();
                                self.gen_expr(value);
                                self.pop_to("%rcx");
                                self.line("mov %rax, (%rcx)");
                            }
                        }
                    }
                    _ => unreachable!("sema guarantees a valid assignment target"),
                }
            }
            Stmt::Expr(e) => {
                self.gen_expr(e);
            }
            Stmt::While(cond, body) => {
                let top = self.new_label();
                let end = self.new_label();
                self.loops.push((top.clone(), end.clone()));
                self.raw(&format!("{top}:"));
                self.gen_branch_false(cond, &end);
                self.gen_block(body);
                self.line(&format!("jmp {top}"));
                self.raw(&format!("{end}:"));
                self.loops.pop();
            }
            Stmt::Loop(body) => {
                let top = self.new_label();
                let end = self.new_label();
                self.loops.push((top.clone(), end.clone()));
                self.raw(&format!("{top}:"));
                self.gen_block(body);
                self.line(&format!("jmp {top}"));
                self.raw(&format!("{end}:"));
                self.loops.pop();
            }
            Stmt::For { var_slot, start, end, end_slot, body, .. } => {
                // Evaluate both bounds once: `var = start`, `end_slot = end`.
                self.gen_expr(start);
                self.store_slot(*var_slot);
                self.gen_expr(end);
                self.store_slot(*end_slot);
                let top = self.new_label();
                let cont = self.new_label();
                let end_l = self.new_label();
                // `continue` must still run the increment, so it targets `cont`.
                self.loops.push((cont.clone(), end_l.clone()));
                self.raw(&format!("{top}:"));
                self.gen_var_operand(*var_slot, "%rax");
                self.gen_var_operand(*end_slot, "%rcx");
                self.line("cmp %rcx, %rax");
                self.line(&format!("jge {end_l}"));
                self.gen_block(body);
                self.raw(&format!("{cont}:"));
                self.gen_var_operand(*var_slot, "%rax");
                self.line("add $1, %rax");
                self.store_slot(*var_slot);
                self.line(&format!("jmp {top}"));
                self.raw(&format!("{end_l}:"));
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
            Stmt::Break => {
                let end = self.loops.last().unwrap().1.clone();
                self.line(&format!("jmp {end}"));
            }
            Stmt::Continue => {
                let top = self.loops.last().unwrap().0.clone();
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

    /// Copy `n` elements of `elem_size` bytes from the address in `%rax` into
    /// the aggregate starting at `slot`. Used to initialise an array from a list
    /// literal, which produces its address rather than its elements.
    fn copy_aggregate(&mut self, slot: i32, n: usize, elem_size: usize) {
        if n == 0 || elem_size != 8 {
            return;
        }
        let base = self.array_off(slot, n);
        if n <= 8 {
            for i in 0..n {
                self.line(&format!("mov {}(%rax), %rdx", i as i32 * 8));
                self.line(&format!("mov %rdx, {}(%rbp)", base + i as i32 * 8));
            }
        } else {
            let l = self.new_label();
            self.line("mov %rax, %rsi");
            self.line(&format!("lea {base}(%rbp), %rdi"));
            self.line(&format!("mov ${n}, %rcx"));
            self.raw(&format!("{l}:"));
            self.line("mov (%rsi), %rdx");
            self.line("mov %rdx, (%rdi)");
            self.line("add $8, %rsi");
            self.line("add $8, %rdi");
            self.line("dec %rcx");
            self.line(&format!("jnz {l}"));
        }
    }

    /// Store `%rax` into `n` consecutive slots starting at `slot`. Used to
    /// zero-fill or splat an array or struct that lives in the frame.
    fn fill_aggregate(&mut self, slot: i32, elem_size: usize, n: usize) {
        if elem_size != 8 {
            // Only 8-byte elements are supported by sema.
            return;
        }
        let base = self.array_off(slot, n);
        if n == 0 {
            return;
        }
        if n <= 8 {
            for i in 0..n {
                self.line(&format!("mov %rax, {}(%rbp)", base + i as i32 * 8));
            }
        } else {
            let l = self.new_label();
            self.line("mov %rax, %r8");
            self.line(&format!("mov ${n}, %rcx"));
            self.line(&format!("lea {base}(%rbp), %rdx"));
            self.raw(&format!("{l}:"));
            self.line("mov %r8, (%rdx)");
            self.line("add $8, %rdx");
            self.line("dec %rcx");
            self.line(&format!("jnz {l}"));
        }
    }

    /// The frame offset of element 0 / field 0 of an aggregate local, or `None`
    /// for a scalar. Aggregates occupy the low end of their slot range, so this
    /// is the *lowest* address of the range and fields index upward from it.
    fn aggregate_base(&self, slot: i32, ty: &Type) -> Option<i32> {
        match ty {
            Type::Array(_, n) => Some(self.array_off(slot, *n)),
            Type::Struct(_, size) => Some(self.array_off(slot, size / 8)),
            _ => None,
        }
    }

    /// A `disp(%rbp)` operand for `base.field` when `base` is a struct local in
    /// memory, or `None` when the base is not a plain local (the caller then
    /// computes the address).
    fn field_operand(&self, base: &Expr, offset: i32) -> Option<String> {
        match base {
            Expr::Var { slot, ty: Type::Struct(_, size), .. } => {
                Some(format!("{}(%rbp)", self.array_off(*slot, size / 8) + offset))
            }
            _ => None,
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
                self.gen_addr(base);
                self.push_rax();
                self.gen_expr(idx);
                self.pop_to("%rcx");
                self.line("lea (%rcx,%rax,8), %rax");
            }
            Expr::Field { base, offset, .. } => {
                self.gen_addr(base);
                if *offset != 0 {
                    self.line(&format!("add ${}, %rax", *offset));
                }
            }
            _ => unreachable!("sema guarantees a valid address expression"),
        }
    }

    fn gen_expr(&mut self, e: &Expr) -> Type {
        match e {
            Expr::Int(v) => {
                self.line(&format!("mov ${v}, %rax"));
                Type::I64
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
                match self.aggregate_base(*slot, ty) {
                    Some(base) => self.line(&format!("lea {base}(%rbp), %rax")),
                    None => match self.reg_of(*slot) {
                        Some(r) => self.line(&format!("mov {r}, %rax")),
                        None => self.line(&format!("mov {}(%rbp), %rax", self.off(*slot))),
                    },
                }
                ty.clone()
            }
            Expr::Field { base, offset, ty, .. } => {
                match self.field_operand(base, *offset) {
                    Some(op) => self.line(&format!("mov {op}, %rax")),
                    None => {
                        self.gen_addr(e);
                        self.line("mov (%rax), %rax");
                    }
                }
                ty.clone()
            }
            Expr::StructLit { name, fields, slot } => {
                // Materialise the literal into its frame buffer and yield that
                // address, which is how every aggregate value is represented.
                let n = fields.len();
                let base = self.array_off(*slot, n);
                for fi in fields {
                    self.gen_expr(&fi.value);
                    self.line(&format!("mov %rax, {}(%rbp)", base + fi.offset));
                }
                self.line(&format!("lea {base}(%rbp), %rax"));
                Type::Struct(name.clone(), n * 8)
            }
            Expr::Index(base, _idx) => {
                let elem = match base.as_ref() {
                    Expr::Var { ty: Type::Array(e, _), .. } => (**e).clone(),
                    _ => Type::I64,
                };
                match self.array_elem_operand(base, _idx) {
                    // `disp(%rbp,%reg,8)` addresses the element in one operand.
                    Some(op) if op.contains("%rax") => {
                        self.gen_expr(_idx);
                        self.line(&format!("mov {op}, %rax"));
                    }
                    Some(op) => {
                        self.line(&format!("mov {op}, %rax"));
                    }
                    None => {
                        self.gen_addr(e);
                        self.line("mov (%rax), %rax");
                    }
                }
                elem
            }
            Expr::Un(op, inner) => {
                self.gen_expr(inner);
                match op {
                    UnOp::Neg => self.line("neg %rax"),
                    UnOp::Not => {
                        self.line("test %rax, %rax");
                        self.line("sete %al");
                        self.line("movzbq %al, %rax");
                    }
                }
                match op {
                    UnOp::Neg => Type::I64,
                    UnOp::Not => Type::Bool,
                }
            }
            Expr::Bin(op, a, b) => self.gen_bin(*op, a, b),
            Expr::Call { name, args, arg_slots, ret_slot, ret_ty } => {
                self.gen_call(name, args, arg_slots, *ret_slot, ret_ty)
            }
            Expr::If(cond, then, els) => {
                let else_l = self.new_label();
                let end_l = self.new_label();
                self.gen_branch_false(cond, &else_l);
                let tt = self.gen_block(then);
                self.line(&format!("jmp {end_l}"));
                self.raw(&format!("{else_l}:"));
                let et = match els {
                    Some(e) => self.gen_block(e),
                    None => {
                        self.line("xor %eax, %eax");
                        Type::Unit
                    }
                };
                self.raw(&format!("{end_l}:"));
                if tt == Type::Unit { et } else { tt }
            }
            Expr::Block(b) => self.gen_block(b),
            Expr::ArrayLit(v, n) => {
                // Only meaningful as a `let` initializer; produce the value.
                let _ = n;
                self.gen_expr(v);
                Type::I64
            }
            Expr::Cast(inner, to) => {
                self.gen_expr(inner);
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
            Expr::Field { base, offset, .. } => self.field_operand(base, *offset),
            Expr::Index(base, idx) => {
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

    fn store_slot(&mut self, slot: i32) {
        match self.reg_of(slot) {
            Some(r) => self.line(&format!("mov %rax, {r}")),
            None => self.line(&format!("mov %rax, {}(%rbp)", self.off(slot))),
        }
    }

    fn gen_bin(&mut self, op: BinOp, a: &Expr, b: &Expr) -> Type {
        use BinOp::*;
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
                self.eval_operands(a, b);
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
            self.eval_operands(a, b);
            self.line("cqto");
            self.line("idiv %rcx");
            if matches!(op, Mod) {
                self.line("mov %rdx, %rax");
            }
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
            // does not, because `c - x` is not `x - c`.
            None if matches!(op, Add | Mul) => match Self::small_imm(a) {
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
                    self.line(&format!("imul ${k}, %rax, %rax"));
                    self.check_overflow();
                }
                _ => unreachable!(),
            }
            return Type::I64;
        }

        self.eval_operands(a, b);
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
            _ => unreachable!(),
        }
        Type::I64
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

    /// Emit `a / d` or `a % d` for a literal divisor `d` using a multiply and a
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
            self.line("neg %rax");
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
                self.line(&format!("movabs ${m}, %rcx"));
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
                self.line("mov %rdx, %rax");
            }
        }
        if matches!(op, BinOp::Mod) {
            self.line("mov %rax, %rdx");
            self.line(&format!("movabs ${d}, %rcx"));
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
    /// array. The caller must have the index value in `%rax`. Returns `None`
    /// for anything else, so the caller falls back to computing the address.
    fn array_elem_operand(&self, base: &Expr, idx: &Expr) -> Option<String> {
        let (slot, n) = match base {
            Expr::Var { slot, ty: Type::Array(_, n), .. } => (*slot, *n),
            _ => return None,
        };
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
                    // Two memory operands: materialise `b`.
                    (false, false) => {
                        self.gen_expr(b);
                        self.line(&format!("cmp %rax, {x}"));
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

    /// Evaluate two operands into the shared convention: left in `%rax`, right
    /// in `%rcx`. Constants and locals are folded in as immediate or memory
    /// operands; anything else goes through the stack.
    fn eval_operands(&mut self, a: &Expr, b: &Expr) {
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
                self.line("mov %rax, %rcx");
                self.line(&format!("mov {la}, %rax"));
            }
            (None, None) => {
                self.gen_expr(a);
                self.push_rax();
                self.gen_expr(b);
                self.line("mov %rax, %rcx");
                self.pop_to("%rax");
            }
        }
    }

    /// Branch to `target` when `cond` is false. Comparisons are emitted as a
    /// direct `cmp`/`jcc` pair, which keeps loop and `if` conditions tight.
    fn gen_branch_false(&mut self, cond: &Expr, target: &str) {
        if let Expr::Bin(op, a, b) = cond {
            if let Some(cc) = invert_cc(*op) {
                if !self.emit_cmp(a, b) {
                    self.eval_operands(a, b);
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
                        self.eval_operands(a, b);
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

    fn gen_call(
        &mut self,
        name: &str,
        args: &[Expr],
        arg_slots: &[(i32, usize)],
        ret_slot: i32,
        ret_ty: &Type,
    ) -> Type {
        const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

        // Aggregate arguments are passed by value: copy each into its hidden
        // frame slot first, so later argument evaluation cannot disturb the
        // source. A copied argument then travels as the address of that slot.
        let mut arg_operand: Vec<Option<String>> = Vec::with_capacity(args.len());
        for (a, (slot, n)) in args.iter().zip(arg_slots.iter()) {
            if *slot >= 0 {
                let base = self.array_off(*slot, *n);
                self.gen_expr(a); // the aggregate's address is in %rax
                self.line("mov %rax, %rsi");
                self.line(&format!("lea {base}(%rbp), %rdi"));
                self.copy_words_rdi_rsi(*n);
                arg_operand.push(Some(format!("{base}(%rbp)")));
            } else {
                arg_operand.push(None);
            }
        }

        // Scalars go into the argument registers. A pointer to an aggregate
        // argument also travels in a register, but its value lives in a frame
        // slot, so it is loaded in the fixed loop below rather than evaluated.
        let direct = arg_operand
            .iter()
            .zip(args.iter())
            .all(|(s, a)| s.is_none() && self.arg_is_simple(a))
            || (args.len() <= 2
                && arg_operand.iter().all(|s| s.is_none())
                && !args.iter().any(Self::contains_call));
        if direct {
            for (a, r) in args.iter().zip(ARG_REGS.iter()) {
                self.gen_expr(a);
                self.line(&format!("mov %rax, {r}"));
            }
        } else {
            // Evaluate scalar arguments right-to-left and push them, so that
            // after popping they land in the ABI registers in order. Aggregate
            // arguments are already copied; their addresses are loaded below.
            let scalars: Vec<&Expr> = args
                .iter()
                .zip(arg_operand.iter())
                .filter(|(_, s)| s.is_none())
                .map(|(a, _)| a)
                .collect();
            for a in scalars.iter().rev() {
                self.gen_expr(a);
                self.push_rax();
            }
            // Pop the scalars into the registers, interleaved with loading the
            // aggregate pointers in the right positions. Walking left to right,
            // a scalar comes off the stack (reverse push order); an aggregate
            // is addressed from its slot.
            let mut stack_scalars: Vec<usize> = Vec::new();
            for (i, s) in arg_operand.iter().enumerate() {
                if s.is_none() {
                    stack_scalars.push(i);
                }
            }
            let mut next_scalar = 0;
            for (i, op) in arg_operand.iter().enumerate() {
                if let Some(addr) = op {
                    self.line(&format!("lea {addr}, {}", ARG_REGS[i]));
                } else {
                    let _ = i;
                    // Pop in left-to-right order; pushes were right-to-left.
                    self.pop_to(ARG_REGS[stack_scalars[next_scalar]]);
                    next_scalar += 1;
                }
            }
        }

        // The return buffer pointer goes in %r10, which is caller-saved and not
        // one of the argument registers, so it survives argument setup.
        if ret_slot >= 0 {
            let n = ret_ty.size() / 8;
            self.line(&format!("lea {}(%rbp), %r10", self.array_off(ret_slot, n)));
        }
        let pad = self.sp_off % 2 != 0;
        if pad {
            self.line("sub $8, %rsp");
        }
        self.line(&format!("call {name}"));
        if pad {
            self.line("add $8, %rsp");
        }
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
            Expr::Bin(_, a, b) => Self::contains_call(a) || Self::contains_call(b),
            Expr::Un(_, a) | Expr::Cast(a, _) => Self::contains_call(a),
            Expr::Index(a, b) => Self::contains_call(a) || Self::contains_call(b),
            Expr::If(c, t, els) => {
                Self::contains_call(c)
                    || Self::block_has_call(t)
                    || els.as_ref().map_or(false, Self::block_has_call)
            }
            Expr::Block(b) => Self::block_has_call(b),
            _ => false,
        }
    }

    fn block_has_call(b: &Block) -> bool {
        b.stmts.iter().any(Self::stmt_has_call)
            || b.tail.as_deref().map_or(false, Self::contains_call)
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
            Stmt::While(..) | Stmt::Loop(..) => true,
            Stmt::Break | Stmt::Continue => false,
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
