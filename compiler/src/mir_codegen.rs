//! Lower the MIR CFG that `mir.rs` parses to x86-64 assembly.
//!
//! This is the *backend* of the `rustc front end + rmc back end` pipeline: rustc
//! performs lexing, parsing, name resolution, type checking and MIR-level
//! optimisation, and this module emits the machine code. The generated assembly
//! is the same dialect `codegen.rs` emits, so it is assembled and linked by the
//! existing in-process `elf.rs` and linked against the existing freestanding
//! `runtime.rs` (no `as`, no `ld`, no libc, no LLVM at the back end).
//!
//! ## Model
//!
//! Every local is assigned a stack slot (`-(n+1)*8(%rbp)`), so a place is a
//! memory operand. A scalar lives in its slot; an aggregate lives in `size`
//! consecutive slots and is named by the address of its first word. A `&T` /
//! `*T` is a scalar holding an address. A projection walks the place:
//! `Field(k)` adds `k*8`, `Index(i)` adds `i*8`, `Deref` loads the pointer.
//!
//! The whole point is speed of both compile and run, so the generator keeps
//! four cheap local-promotion rules rather than a full register allocator:
//!
//! * the induction local of a counted loop lives in a callee-saved register for
//!   the whole function (it is written once per iteration);
//! * a local mentioned at least twice and never projected lives in a register;
//! * an `x = x <op> k` accumulation updates its register in place;
//! * a comparison feeds `cmp`/`jcc` directly, so `bool` temporaries never hit
//!   memory.

use crate::mir::*;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Callee-saved registers the generator may hand to a promoted local.
const PROMOTE_REGS: [&str; 5] = ["%rbx", "%r12", "%r13", "%r14", "%r15"];

pub struct Options {
    /// Emit `jo __rmc_overflow` after `+`/`-`/`*` (rustc's overflow-checks).
    pub overflow_checks: bool,
    /// Promote hot locals into callee-saved registers (on by default).
    pub promote: bool,
    /// Align loop headers to a 16-byte boundary (on by default).
    pub align_loops: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { overflow_checks: false, promote: true, align_loops: true }
    }
}

/// A promoted local: which register holds it, and whether that register has to
/// be saved/restored across a call.
#[derive(Clone, Copy)]
struct Promo {
    reg: &'static str,
    caller_saved: bool,
}

struct FnGen<'a> {
    prog: &'a Program,
    func: &'a Function,
    opts: &'a Options,
    /// String literal content -> its `.rodata` label.
    string_ids: &'a HashMap<String, String>,
    /// Local index -> its byte size (`8` for a scalar, `8*n` for an aggregate).
    size: Vec<i64>,
    /// Local index -> type.
    ty: Vec<Ty>,
    /// Local index -> promoted register, when promoted.
    promo: HashMap<u32, Promo>,
    /// Local index -> shadow slot, for a promoted local whose address is taken.
    shadow: HashMap<u32, i64>,
    /// Locals that are an operand of a loop's counted comparison (the counter
    /// and its bound), promoted first.
    loop_operands: Vec<u32>,
    /// Callee-saved registers saved below `%rbp`; frame slots start past them.
    saved_regs: i64,
    next_slot: i64,
    out: String,
    label: u64,
    errors: Vec<String>,
}

/// Generate x86-64 assembly for `prog`, ready to concatenate with the runtime.
pub fn compile(prog: &Program, opts: &Options) -> Result<String, String> {
    let mut asm = String::new();
    asm.push_str(".text\n");

    // Emit only functions the program can reach from `main` (dead-function
    // elimination keeps the artifact small and the compile fast).
    let live = reachable(prog);
    let order: Vec<&Function> = prog.funcs.iter().filter(|f| live.contains(&f.name)).collect();

    // String literals become `.rodata` labels, emitted once.
    let (strings, string_ids) = collect_strings(prog);

    for f in &order {
        if is_unsupported_builtin(&f.name) {
            continue;
        }
        let mut g = FnGen::new(prog, f, opts, &string_ids);
        let body = g.gen_fn()?;
        asm.push_str(&body);
        if !g.errors.is_empty() {
            return Err(g.errors.join("\n"));
        }
    }

    if !strings.is_empty() {
        asm.push_str("\n.section .rodata\n");
        for s in &strings {
            let label = &string_ids[s];
            let _ = writeln!(asm, "{label}:");
            let mut parts = Vec::new();
            for b in s.bytes() {
                parts.push(b.to_string());
            }
            parts.push("0".to_string()); // NUL-terminate
            let _ = writeln!(asm, "  .byte {}", parts.join(", "));
        }
    }
    Ok(asm)
}

/// Functions reachable from `main` by direct calls.
fn reachable(prog: &Program) -> HashSet<String> {
    let mut live = HashSet::new();
    let Some(main) = prog.func("main") else { return live };
    let mut stack = vec![main.name.clone()];
    while let Some(name) = stack.pop() {
        if !live.insert(name.clone()) {
            continue;
        }
        if let Some(f) = prog.func(&name) {
            for b in &f.blocks {
                if let Terminator::Call { func: CallTarget::Direct(callee), .. } = &b.term {
                    if prog.func(callee).is_some() {
                        stack.push(callee.clone());
                    }
                }
            }
        }
    }
    live
}

fn is_unsupported_builtin(name: &str) -> bool {
    name.contains("::") || name.starts_with('<') || name == "rust_begin_unwind"
}

/// The System V integer argument registers, in order.
const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];

/// Every distinct string literal in the program, in first-seen order, and a map
/// from its content to a `.rodata` label.
fn collect_strings(prog: &Program) -> (Vec<String>, HashMap<String, String>) {
    let mut order = Vec::new();
    let mut map = HashMap::new();
    for f in &prog.funcs {
        for b in &f.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    collect_strings_rv(rv, &mut order, &mut map);
                }
            }
        }
    }
    (order, map)
}

fn collect_strings_rv(rv: &Rvalue, order: &mut Vec<String>, map: &mut HashMap<String, String>) {
    let mut add = |c: &Const| {
        if let Const::Str(s) = c {
            if !map.contains_key(s) {
                map.insert(s.clone(), format!("__rmc_lit{}", map.len()));
                order.push(s.clone());
            }
        }
    };
    match rv {
        Rvalue::Use(op) => {
            if let Operand::Const(c) = op {
                add(c)
            }
        }
        Rvalue::Binary(_, a, b) => {
            if let Operand::Const(c) = a {
                add(c)
            }
            if let Operand::Const(c) = b {
                add(c)
            }
        }
        Rvalue::Unary(_, a) => {
            if let Operand::Const(c) = a {
                add(c)
            }
        }
        Rvalue::Cast { op, .. } => {
            if let Operand::Const(c) = op {
                add(c)
            }
        }
        Rvalue::Repeat { op, .. } => {
            if let Operand::Const(c) = op {
                add(c)
            }
        }
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => {
            for op in ops {
                if let Operand::Const(c) = op {
                    add(c)
                }
            }
        }
        _ => {}
    }
}

impl<'a> FnGen<'a> {
    fn new(
        prog: &'a Program,
        func: &'a Function,
        opts: &'a Options,
        string_ids: &'a HashMap<String, String>,
    ) -> Self {
        FnGen {
            prog,
            func,
            opts,
            string_ids,
            size: Vec::new(),
            ty: Vec::new(),
            promo: HashMap::new(),
            shadow: HashMap::new(),
            loop_operands: Vec::new(),
            saved_regs: 0,
            next_slot: func.locals.len() as i64,
            out: String::new(),
            label: 0,
            errors: Vec::new(),
        }
    }

    fn emit(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn new_label(&mut self) -> String {
        self.label += 1;
        format!(".Lrmc{}", self.label)
    }

    fn alloc_slot(&mut self) -> i64 {
        let s = self.next_slot;
        self.next_slot += 1;
        s
    }

    fn off(&self, slot: i64) -> String {
        format!("{}(%rbp)", -(slot + 1 + self.saved_regs) * 8)
    }

    fn local_size(&self, local: u32) -> i64 {
        self.size.get(local as usize).copied().unwrap_or(8)
    }

    fn local_ty(&self, local: u32) -> Ty {
        self.ty.get(local as usize).cloned().unwrap_or(Ty::I64)
    }

    /// A scalar local's home: its promoted register or its frame slot.
    fn local_operand(&self, local: u32) -> String {
        match self.promo.get(&local) {
            Some(p) => p.reg.to_string(),
            None => self.off(local as i64),
        }
    }

    /// True when a scalar local is held in a register (so `p.proj` is empty and
    /// reading it needs no memory access).
    fn place_is_promoted(&self, p: &Place) -> bool {
        p.proj.is_empty() && self.promo.contains_key(&p.local)
    }

    // --- analysis ----------------------------------------------------------

    /// Compute each local's size and type, and find the operands of counted
    /// loop tests (`switchInt(Lt(counter, bound))`).
    fn analyze(&mut self) {
        for l in &self.func.locals {
            self.ty.push((*l.ty).clone());
            self.size.push(ty_size(&l.ty));
        }
        // For each `switchInt(cmp) -> [0: exit, otherwise: body]`, the `cmp` is
        // a comparison assignment in the same block; its operands are the loop
        // counter and bound.
        for b in &self.func.blocks {
            let Terminator::SwitchInt { discr, targets, .. } = &b.term else { continue };
            if targets.len() != 1 {
                continue;
            }
            let Some(pl) = operand_place(discr) else { continue };
            if !pl.proj.is_empty() {
                continue;
            }
            for s in &b.stmts {
                if let Stmt::Assign(p, Rvalue::Binary(op, a, c)) = s {
                    if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                        for op_ in [a, c] {
                            if let Operand::Copy(q) | Operand::Move(q) = op_ {
                                if q.proj.is_empty() {
                                    self.loop_operands.push(q.local);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Promote the operands of counted loop tests, then the hottest remaining
    /// scalars, into callee-saved registers.
    fn plan_promotion(&mut self) {
        if !self.opts.promote {
            return;
        }
        let mut used: HashSet<&'static str> = HashSet::new();
        // Loop counters and bounds first: they are touched every iteration.
        for l in self.loop_operands.clone() {
            self.promote_local(l, &mut used);
        }
        // Then the hottest remaining scalar locals. A local with a projection
        // (a struct field, an array element) is left in memory: promoting it
        // would mean tracking its address, which the cheap rules here do not.
        let mut counts: HashMap<u32, u32> = HashMap::new();
        let mut projected: HashSet<u32> = HashSet::new();
        let mut addr_taken: HashSet<u32> = HashSet::new();
        for b in &self.func.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(p, rv) = s {
                    if !p.proj.is_empty() {
                        projected.insert(p.local);
                    }
                    scan_rv(rv, &mut counts, &mut projected, &mut addr_taken);
                }
            }
            scan_term(&b.term, &mut counts, &mut projected, &mut addr_taken);
        }
        let mut ranked: Vec<(u32, u32)> = counts
            .into_iter()
            .filter(|(l, c)| *c >= 2 && !projected.contains(l) && !addr_taken.contains(l))
            .filter(|(l, _)| !self.promo.contains_key(l))
            .filter(|(l, _)| !self.size.get(*l as usize).copied().unwrap_or(8).eq(&8) || {
                !self.ty.get(*l as usize).map(|t| t.is_aggregate()).unwrap_or(false)
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (l, _) in ranked {
            if self.promo.len() >= PROMOTE_REGS.len() {
                break;
            }
            self.promote_local(l, &mut used);
        }
    }

    fn promote_local(&mut self, local: u32, used: &mut HashSet<&'static str>) {
        if self.promo.contains_key(&local) {
            return;
        }
        let ty = self.ty.get(local as usize).cloned().unwrap_or(Ty::I64);
        if ty.is_aggregate() {
            return;
        }
        let Some(reg) = PROMOTE_REGS.iter().find(|r| !used.contains(*r)).copied() else {
            return;
        };
        used.insert(reg);
        // A local that survives a call needs its register saved, which a
        // `%rbx`/`%r12`-`%r15` (callee-saved) register gets for free; the
        // generator saves every register it promotes in the prologue anyway,
        // so a call site never has to spill.
        let _ = local;
        self.promo.insert(local, Promo { reg, caller_saved: false });
    }

    // --- function ----------------------------------------------------------

    fn gen_fn(&mut self) -> Result<String, String> {
        self.analyze();
        self.plan_promotion();
        // A shadow slot for each promoted local, so taking its address has a
        // stable home. Allocated once, before the frame is sized.
        for l in 0..self.func.locals.len() as u32 {
            if self.promo.contains_key(&l) {
                let s = self.alloc_slot();
                self.shadow.insert(l, s);
            }
        }
        let base_slots = self.next_slot;

        // Dry pass: discover how many spill slots the body needs, then run the
        // real pass with the frame sized to include them. The two passes
        // allocate slots identically, so the reservation is exact.
        self.gen_body()?;
        let frame_slots = self.next_slot;
        self.out.clear();
        self.label = 0;
        self.next_slot = base_slots;

        // Save the promoted registers (callee-saved: the ABI requires it).
        let saved: Vec<&'static str> = PROMOTE_REGS
            .iter()
            .copied()
            .filter(|r| self.promo.values().any(|p| p.reg == *r))
            .collect();
        // The saved registers sit below `%rbp`, so every frame slot is shifted
        // past them; `off` reads this.
        self.saved_regs = saved.len() as i64;
        let _ = writeln!(self.out, ".globl {}", self.func.name);
        let _ = writeln!(self.out, "{}:", self.func.name);
        self.out.push_str("  push %rbp\n  mov %rsp, %rbp\n");
        for r in &saved {
            let _ = writeln!(self.out, "  push {r}");
        }
        // Keep `%rsp` 16-byte aligned at every call: the saved registers plus
        // the frame must together be a multiple of 16.
        let saved_bytes = self.saved_regs * 8;
        let frame_bytes = align_up(frame_slots * 8 + saved_bytes, 16) - saved_bytes;
        if frame_bytes > 0 {
            let _ = writeln!(self.out, "  sub ${frame_bytes}, %rsp");
        }

        self.gen_body()?;

        // Epilogue: restore the saved registers and return `%rax`. The saved
        // registers live directly below `%rbp`, but `%rsp` is still at the
        // bottom of the frame, so it has to be moved back up first.
        let _ = writeln!(self.out, ".Lret_{}:", self.func.name);
        if saved_bytes > 0 {
            let _ = writeln!(self.out, "  lea -{saved_bytes}(%rbp), %rsp");
        }
        for r in saved.iter().rev() {
            let _ = writeln!(self.out, "  pop {r}");
        }
        self.out.push_str("  leave\n  ret\n");
        Ok(std::mem::take(&mut self.out))
    }

    /// Argument setup plus every basic block. Run twice (dry, then real).
    fn gen_body(&mut self) -> Result<(), String> {
        // MIR numbers arguments from `_1`, so the register is chosen by the
        // argument's ordinal, not by its local index.
        let mut args: Vec<u32> = (0..self.func.locals.len() as u32)
            .filter(|i| self.func.locals[*i as usize].is_arg)
            .collect();
        args.sort_unstable();
        for (ord, li) in args.into_iter().enumerate() {
            if ord >= ARG_REGS.len() {
                return Err(format!("mir_codegen: {} takes more than 6 arguments", self.func.name));
            }
            let dst = self.local_operand(li);
            let _ = writeln!(self.out, "  mov {}, {}", ARG_REGS[ord], dst);
        }
        for i in 0..self.func.blocks.len() as u32 {
            let _ = writeln!(self.out, ".Lbb{}:", self.bb_label(i));
            self.gen_block(i)?;
        }
        Ok(())
    }

    /// A basic-block label, qualified by the function name so that labels from
    /// different functions cannot collide (assembler-local `.L` labels share
    /// one namespace across the whole translation unit).
    fn bb_label(&self, bb: u32) -> String {
        let mut f = String::with_capacity(self.func.name.len() + 8);
        for c in self.func.name.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                f.push(c);
            } else {
                f.push('_');
            }
        }
        format!("{f}_{bb}")
    }

    fn gen_block(&mut self, bb: u32) -> Result<(), String> {
        let block = &self.func.blocks[bb as usize];
        let stmts = block.stmts.clone();
        let term = block.term.clone();
        // A `switchInt` on a comparison folds the comparison into a `jcc`; the
        // statement that computed the `bool` is then dead, so skip it.
        let skip = self.compare_branch_skip(bb, &term);
        for (i, s) in stmts.iter().enumerate() {
            if Some(i) == skip {
                continue;
            }
            if let Stmt::Assign(p, rv) = s {
                self.gen_assign(p, rv)?;
            }
        }
        self.gen_term(bb, &term)
    }

    /// The index of the statement a comparison switch folds away, if any.
    fn compare_branch_skip(&self, bb: u32, term: &Terminator) -> Option<usize> {
        let Terminator::SwitchInt { discr, targets, .. } = term else { return None };
        if targets.len() != 1 {
            return None;
        }
        let pl = operand_place(discr)?;
        if !pl.proj.is_empty() {
            return None;
        }
        let block = self.func.blocks.get(bb as usize)?;
        let mut last = None;
        for (i, s) in block.stmts.iter().enumerate() {
            if let Stmt::Assign(p, Rvalue::Binary(op, _, _)) = s {
                if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                    last = Some(i);
                }
            }
        }
        last
    }

    fn gen_term(&mut self, bb: u32, term: &Terminator) -> Result<(), String> {
        match term {
            Terminator::Goto(t) => {
                self.emit(&format!("  jmp .Lbb{}", self.bb_label(*t)));
            }
            Terminator::Return => {
                // A non-unit return value lives in `_0`.
                if self.local_ty(0) != Ty::Unit && !self.func.locals.is_empty() {
                    let src = self.local_operand(0);
                    if src != "%rax" {
                        self.emit(&format!("  mov {src}, %rax"));
                    }
                }
                self.emit(&format!("  jmp .Lret_{}", self.func.name));
            }
            Terminator::Unreachable => {
                self.emit(&format!("  jmp .Lret_{}", self.func.name));
            }
            Terminator::Abort => {
                // A trap: exit 101, matching rustc's panic status.
                self.emit("  mov $101, %rdi");
                self.emit("  mov $60, %rax");
                self.emit("  syscall");
            }
            Terminator::SwitchInt { discr, targets, otherwise } => {
                self.gen_switch(bb, discr, targets, *otherwise)?;
            }
            Terminator::Call { dest, func, args, target } => {
                self.gen_call(dest, func, args)?;
                let t = target.unwrap_or_else(|| {
                    // A call with no return edge is `-> []` (noreturn).
                    u32::MAX
                });
                if t == u32::MAX {
                    self.emit(&format!("  jmp .Lret_{}", self.func.name));
                } else {
                    self.emit(&format!("  jmp .Lbb{}", self.bb_label(t)));
                }
            }
        }
        Ok(())
    }

    fn gen_switch(
        &mut self,
        bb: u32,
        discr: &Operand,
        targets: &[(i64, u32)],
        otherwise: u32,
    ) -> Result<(), String> {
        // A two-way `Lt/Le/Gt/Ge` switch is a loop/if test: emit a `cmp` plus
        // the matching `jcc` straight to the taken block.
        if targets.len() == 1 && self.try_branch_compare(bb, discr, targets[0].1, otherwise)? {
            return Ok(());
        }
        let (src, is_mem) = self.eval_operand(discr)?;
        if is_mem {
            self.emit(&format!("  mov {src}, %rax"));
            for (val, t) in targets {
                self.emit(&format!("  cmp ${val}, %rax"));
                self.emit(&format!("  je .Lbb{}", self.bb_label(*t)));
            }
        } else {
            for (val, t) in targets {
                self.emit(&format!("  cmp ${val}, {src}"));
                self.emit(&format!("  je .Lbb{}", self.bb_label(*t)));
            }
        }
        self.emit(&format!("  jmp .Lbb{}", self.bb_label(otherwise)));
        Ok(())
    }

    /// Try to emit a `switchInt(bool) -> [0: false_target, otherwise: true_target]`
    /// as one `cmp`+`jcc`, folding the comparison away entirely. Returns false
    /// when the discriminant is not a comparison.
    fn try_branch_compare(
        &mut self,
        bb: u32,
        discr: &Operand,
        false_target: u32,
        true_target: u32,
    ) -> Result<bool, String> {
        let Some(pl) = operand_place(discr) else { return Ok(false) };
        if !pl.proj.is_empty() {
            return Ok(false);
        }
        // Find the statement that assigned this local a comparison.
        let Some(block) = self.func.blocks.get(bb as usize) else { return Ok(false) };
        let mut cmp: Option<(BinOp, Operand, Operand)> = None;
        for s in &block.stmts {
            if let Stmt::Assign(p, Rvalue::Binary(op, a, b)) = s {
                if p.local == pl.local && p.proj.is_empty() && is_compare(*op) {
                    cmp = Some((*op, a.clone(), b.clone()));
                }
            }
        }
        let Some((op, a, b)) = cmp else { return Ok(false) };
        self.emit_cmp(op, &a, &b)?;
        // The comparison being *true* selects `otherwise`; being false selects
        // the `0:` target.
        self.emit(&format!("  {} .Lbb{}", jcc(op), self.bb_label(true_target)));
        self.emit(&format!("  jmp .Lbb{}", self.bb_label(false_target)));
        Ok(true)
    }

    // --- operands and places ----------------------------------------------

    /// Evaluate a scalar operand to something usable as an instruction operand:
    /// a register, an immediate, or a direct memory slot. Returns the operand
    /// text and whether it is a memory operand. A complex operand is loaded
    /// into `%rax`.
    fn eval_operand(&mut self, op: &Operand) -> Result<(String, bool), String> {
        match op {
            Operand::Const(c) => Ok((self.const_operand(c)?, false)),
            Operand::Copy(p) | Operand::Move(p) => {
                if self.place_is_simple(p) {
                    let mem = !self.promo.contains_key(&p.local);
                    Ok((self.local_operand(p.local), mem))
                } else {
                    self.load_place_to(p, "%rax")?;
                    Ok(("%rax".to_string(), false))
                }
            }
        }
    }

    /// Evaluate a scalar operand *without* clobbering `%rax`: a simple operand
    /// is returned directly, a complex one is loaded into `%rcx`.
    fn eval_operand_keep_rax(&mut self, op: &Operand) -> Result<(String, bool), String> {
        match op {
            Operand::Const(c) => Ok((self.const_operand(c)?, false)),
            Operand::Copy(p) | Operand::Move(p) => {
                if self.place_is_simple(p) {
                    let mem = !self.promo.contains_key(&p.local);
                    Ok((self.local_operand(p.local), mem))
                } else {
                    self.load_place_to(p, "%rcx")?;
                    Ok(("%rcx".to_string(), false))
                }
            }
        }
    }

    /// True when an operand names a scalar local or constant, so reading it
    /// needs no code and cannot clobber a register.
    fn place_is_simple(&self, p: &Place) -> bool {
        p.proj.is_empty() && !self.local_ty(p.local).is_aggregate()
    }

    fn operand_is_simple(&self, op: &Operand) -> bool {
        match op {
            Operand::Const(_) => true,
            Operand::Copy(p) | Operand::Move(p) => self.place_is_simple(p),
        }
    }

    fn const_operand(&mut self, c: &Const) -> Result<String, String> {
        Ok(match c {
            Const::Int(n) => format!("${n}"),
            Const::Bool(b) => format!("${}", *b as i64),
            Const::Unit => "$0".to_string(),
            Const::Str(s) => {
                let label = self
                    .string_ids
                    .get(s)
                    .cloned()
                    .unwrap_or_else(|| s.clone());
                format!("${label}")
            }
            Const::Named(n) => {
                return Err(format!("mir_codegen: unsupported constant `{n}`"));
            }
        })
    }

    /// Leave the address of `place` in `%rax`.
    fn place_addr(&mut self, p: &Place) -> Result<(), String> {
        let starts_deref = matches!(p.proj.first(), Some(Proj::Deref));
        if starts_deref {
            let src = self.local_operand(p.local);
            self.emit(&format!("  mov {src}, %rax"));
        } else if p.proj.is_empty() {
            // A promoted scalar has no address in memory; materialise it into
            // its shadow slot so the address is stable.
            if self.promo.contains_key(&p.local) {
                let reg = self.promo[&p.local].reg;
                let slot = self.shadow[&p.local];
                self.emit(&format!("  mov {reg}, {}", self.off(slot)));
                self.emit(&format!("  lea {}, %rax", self.off(slot)));
            } else {
                self.emit(&format!("  lea {}, %rax", self.off(p.local as i64)));
            }
        } else {
            self.emit(&format!("  lea {}, %rax", self.off(p.local as i64)));
        }
        let rest = if starts_deref { &p.proj[1..] } else { &p.proj[..] };
        for proj in rest {
            match proj {
                Proj::Field(k) => self.emit(&format!("  add ${}, %rax", *k as i64 * 8)),
                Proj::IndexConst(i) => self.emit(&format!("  add ${}, %rax", *i as i64 * 8)),
                Proj::IndexLocal(l) => {
                    let src = self.local_operand(*l);
                    self.emit(&format!("  mov {src}, %rcx"));
                    self.emit("  lea (%rax,%rcx,8), %rax");
                }
                Proj::Deref => self.emit("  mov (%rax), %rax"),
                Proj::RawRef => {}
            }
        }
        Ok(())
    }

    /// Leave the scalar value at `place` in `%rax`.
    fn load_place_value(&mut self, p: &Place) -> Result<(), String> {
        self.load_place_to(p, "%rax")
    }

    /// Load the scalar value at `place` into `reg` (which may be `%rax`).
    fn load_place_to(&mut self, p: &Place, reg: &str) -> Result<(), String> {
        self.place_addr(p)?;
        if reg != "%rax" {
            self.emit(&format!("  mov (%rax), {reg}"));
        } else {
            self.emit("  mov (%rax), %rax");
        }
        Ok(())
    }

    /// Evaluate an operand that names an aggregate, leaving its address in
    /// `%rax`.
    fn eval_aggregate_addr(&mut self, op: &Operand) -> Result<(), String> {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.place_addr(p),
            Operand::Const(_) => Err("mir_codegen: aggregate constant unsupported".into()),
        }
    }

    /// Copy `size` bytes from the address in `%rsi` to the address in `%rdi`.
    fn copy_words(&mut self, size: i64) {
        if size <= 0 {
            return;
        }
        if size <= 64 {
            let n = size / 8;
            for i in 0..n {
                let d = i * 8;
                self.emit(&format!("  mov {d}(%rsi), %rcx"));
                self.emit(&format!("  mov %rcx, {d}(%rdi)"));
            }
        } else {
            self.emit("  mov %rdi, %rdx");
            self.emit("  mov %rsi, %r8");
            self.emit("  mov %rdx, %rdi");
            self.emit("  mov %r8, %rsi");
            self.emit(&format!("  mov ${}, %rcx", size / 8));
            let loop_l = self.new_label();
            self.emit(&format!("{loop_l}:"));
            self.emit("  mov (%rsi), %rax");
            self.emit("  mov %rax, (%rdi)");
            self.emit("  add $8, %rsi");
            self.emit("  add $8, %rdi");
            self.emit("  dec %rcx");
            self.emit(&format!("  jnz {loop_l}"));
        }
    }

    // --- assignment --------------------------------------------------------

    fn gen_assign(&mut self, dest: &Place, rv: &Rvalue) -> Result<(), String> {
        match rv {
            Rvalue::Use(op) => {
                let ty = self.place_ty(dest);
                if ty.is_aggregate() {
                    // Copy the aggregate words into the destination.
                    let size = self.place_size(dest);
                    self.eval_aggregate_addr(op)?;
                    self.emit("  mov %rax, %rsi");
                    self.place_addr(dest)?;
                    self.emit("  mov %rax, %rdi");
                    self.copy_words(size);
                } else {
                    let (src, _) = self.eval_operand(op)?;
                    self.store_scalar(dest, &src)?;
                }
            }
            Rvalue::Binary(op, a, b) => self.gen_binary(dest, *op, a, b)?,
            Rvalue::Unary(op, a) => {
                let (src, _) = self.eval_operand(a)?;
                if src != "%rax" {
                    self.emit(&format!("  mov {src}, %rax"));
                }
                match op {
                    UnOp::Not => self.emit("  xor $1, %rax"),
                    UnOp::Neg => self.emit("  neg %rax"),
                    UnOp::PtrMetadata => {
                        return Err("mir_codegen: PtrMetadata (slice length) unsupported".into())
                    }
                }
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Cast { op, .. } => {
                let (src, _) = self.eval_operand(op)?;
                self.store_scalar(dest, &src)?;
            }
            Rvalue::Ref { place, .. } | Rvalue::RawRef { place, .. } => {
                self.place_addr(place)?;
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Repeat { op, count } => {
                let (src, _) = self.eval_operand(op)?;
                if src != "%rax" {
                    self.emit(&format!("  mov {src}, %rax"));
                }
                // `[v; n]`: store v into each of the n words.
                for i in 0..*count {
                    let target = self.subplace(dest, i);
                    self.store_scalar(&target, "%rax")?;
                }
            }
            Rvalue::Array(ops) | Rvalue::Aggregate(ops) => {
                for (i, op) in ops.iter().enumerate() {
                    let target = self.subplace(dest, i as u64);
                    let ty = self.place_ty(&target);
                    if ty.is_aggregate() {
                        let size = self.place_size(&target);
                        self.eval_aggregate_addr(op)?;
                        self.emit("  mov %rax, %rsi");
                        self.place_addr(&target)?;
                        self.emit("  mov %rax, %rdi");
                        self.copy_words(size);
                    } else {
                        let (src, _) = self.eval_operand(op)?;
                        self.store_scalar(&target, &src)?;
                    }
                }
            }
            Rvalue::Len(place) => {
                let len = self.array_len(place);
                self.emit(&format!("  mov ${len}, %rax"));
                self.store_scalar(dest, "%rax")?;
            }
            Rvalue::Unsupported(s) => {
                return Err(format!("mir_codegen: unsupported rvalue `{s}`"));
            }
        }
        Ok(())
    }

    /// A sub-place for aggregate element/field `i`.
    fn subplace(&self, dest: &Place, i: u64) -> Place {
        let mut p = dest.clone();
        // A field and an array element are both `+ i*8`; `Field` reads better
        // for a struct literal, `IndexConst` for an array.
        p.proj.push(Proj::IndexConst(i));
        p
    }

    /// Store the scalar in `src` (a register, immediate, or frame slot) at
    /// `dest`.
    fn store_scalar(&mut self, dest: &Place, src: &str) -> Result<(), String> {
        if dest.proj.is_empty() && self.promo.contains_key(&dest.local) {
            let dst = self.promo[&dest.local].reg;
            if src == dst {
                return Ok(());
            }
            self.emit(&format!("  mov {src}, {dst}"));
            return Ok(());
        }
        // A memory source cannot be the source of a memory destination, and
        // `%rax` is clobbered by `place_addr`, so both go through `%rcx`.
        let is_mem = src.contains('(');
        let src = if src == "%rax" || is_mem {
            self.emit(&format!("  mov {src}, %rcx"));
            "%rcx"
        } else {
            src
        };
        self.place_addr(dest)?;
        self.emit(&format!("  mov {src}, (%rax)"));
        Ok(())
    }

    fn place_ty(&self, p: &Place) -> Ty {
        let mut ty = self.local_ty(p.local);
        for proj in &p.proj {
            ty = match proj {
                Proj::Field(k) => match &ty {
                    Ty::Tuple(tys) => tys.get(*k as usize).map(|t| (**t).clone()).unwrap_or(Ty::I64),
                    _ => Ty::I64,
                },
                Proj::IndexConst(_) | Proj::IndexLocal(_) => match &ty {
                    Ty::Array(elem, _) => (**elem).clone(),
                    _ => Ty::I64,
                },
                Proj::Deref => match &ty {
                    Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                    _ => Ty::I64,
                },
                Proj::RawRef => ty.clone(),
            };
        }
        ty
    }

    fn place_size(&self, p: &Place) -> i64 {
        ty_size(&self.place_ty(p))
    }

    fn array_len(&self, p: &Place) -> u64 {
        let mut ty = self.local_ty(p.local);
        for proj in &p.proj {
            if let Proj::Deref = proj {
                ty = match &ty {
                    Ty::Ref(t) | Ty::Ptr(t) => (**t).clone(),
                    _ => ty.clone(),
                };
            }
        }
        match ty {
            Ty::Array(_, n) => n,
            _ => 0,
        }
    }

    fn gen_binary(
        &mut self,
        dest: &Place,
        op: BinOp,
        a: &Operand,
        b: &Operand,
    ) -> Result<(), String> {
        match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.emit_cmp(op, a, b)?;
                self.emit(&format!("  {} %al", setcc(op)));
                self.emit("  movzbl %al, %eax");
                self.store_scalar(dest, "%rax")?;
            }
            BinOp::Cmp => return Err("mir_codegen: three-way Cmp unsupported".into()),
            BinOp::Offset | BinOp::PtrOffset => {
                // `a + b*8` — a pointer offset by b elements.
                let (av, _) = self.eval_operand(a)?;
                if av != "%rax" {
                    self.emit(&format!("  mov {av}, %rax"));
                }
                let (bv, _) = self.eval_operand(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                self.emit("  lea (%rax,%rcx,8), %rax");
                self.store_scalar(dest, "%rax")?;
            }
            _ => {
                self.emit_arith(op, a, b)?;
                self.store_scalar(dest, "%rax")?;
            }
        }
        Ok(())
    }

    /// Emit `a op b` into `%rax`.
    fn emit_arith(&mut self, op: BinOp, a: &Operand, b: &Operand) -> Result<(), String> {
        // Evaluate the left operand into `%rax` unless it is a bare simple
        // operand, which needs no code and can be read later.
        let (av, a_mem) = self.eval_operand(a)?;
        if av != "%rax" {
            self.emit(&format!("  mov {av}, %rax"));
        }
        let _ = a_mem;
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                let (bv, _) = self.eval_operand_keep_rax(b)?;
                let m = match op {
                    BinOp::Add => "add",
                    BinOp::Sub => "sub",
                    BinOp::Mul => "imul",
                    BinOp::BitAnd => "and",
                    BinOp::BitOr => "or",
                    _ => "xor",
                };
                self.emit(&format!("  {m} {bv}, %rax"));
                if self.opts.overflow_checks && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) {
                    self.emit("  jo __rmc_overflow");
                }
            }
            BinOp::Shl | BinOp::Shr => {
                // The shift count must be in `%cl`. Save `%rax` (the value) in
                // `%rdx` while the count is evaluated, then move it to `%rcx`.
                self.emit("  mov %rax, %rdx");
                let (bv, _) = self.eval_operand(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                self.emit("  mov %rdx, %rax");
                let m = if op == BinOp::Shl { "sal" } else { "sar" };
                self.emit(&format!("  {m} %cl, %rax"));
            }
            BinOp::Div | BinOp::Rem => {
                let (bv, _) = self.eval_operand(b)?;
                if bv != "%rcx" {
                    self.emit(&format!("  mov {bv}, %rcx"));
                }
                self.emit("  cqto");
                self.emit("  idiv %rcx");
                if op == BinOp::Rem {
                    self.emit("  mov %rdx, %rax");
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    /// Emit `cmp` so that the flags read as `a op b` (i.e. `a - b`).
    fn emit_cmp(&mut self, _op: BinOp, a: &Operand, b: &Operand) -> Result<(), String> {
        if self.operand_is_simple(a) && self.operand_is_simple(b) {
            let (av, _) = self.eval_operand(a)?;
            let (bv, _) = self.eval_operand(b)?;
            self.emit(&format!("  cmp {bv}, {av}"));
            return Ok(());
        }
        // Load `a` into `%rax`, keep it while `b` is evaluated into `%rcx`,
        // then `cmp %rcx, %rax`.
        let (av, _) = self.eval_operand(a)?;
        if av != "%rax" {
            self.emit(&format!("  mov {av}, %rax"));
        }
        let (bv, _) = self.eval_operand_keep_rax(b)?;
        self.emit(&format!("  cmp {bv}, %rax"));
        Ok(())
    }

    // --- calls -------------------------------------------------------------

    fn gen_call(&mut self, dest: &Place, func: &CallTarget, args: &[Operand]) -> Result<(), String> {
        let name = match func {
            CallTarget::Direct(n) => n.clone(),
            CallTarget::Indirect => {
                return Err("mir_codegen: indirect call (fn pointer) unsupported".into())
            }
        };
        if name == "alloc" {
            // rustc models `alloc` as an unsafe fn with an `Option`-like ABI;
            // the program's own `alloc` is rmc's builtin, so call it directly.
        }
        // Evaluate arguments left to right. An aggregate is passed by address;
        // a scalar in a frame slot or an immediate is stable and used directly;
        // only a value that landed in `%rax`/`%rcx` is stashed in a spill slot,
        // because evaluating a later argument may clobber it.
        let mut reg_ops: Vec<(usize, String)> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if i >= ARG_REGS.len() {
                return Err(format!("mir_codegen: call to {name} with more than 6 arguments"));
            }
            if self.operand_ty(arg).is_aggregate() {
                self.eval_aggregate_addr(arg)?;
                let tmp = self.alloc_slot();
                self.emit(&format!("  mov %rax, {}", self.off(tmp)));
                reg_ops.push((i, self.off(tmp)));
            } else {
                let (v, _) = self.eval_operand(arg)?;
                let stable = v.starts_with('$') || v.contains('(');
                if stable {
                    reg_ops.push((i, v));
                } else {
                    let tmp = self.alloc_slot();
                    self.emit(&format!("  mov {v}, {}", self.off(tmp)));
                    reg_ops.push((i, self.off(tmp)));
                }
            }
        }
        for (i, v) in &reg_ops {
            if ARG_REGS[*i] != *v {
                self.emit(&format!("  mov {v}, {}", ARG_REGS[*i]));
            }
        }
        self.emit(&format!("  call {name}"));
        if self.place_ty(dest).is_aggregate() {
            // Aggregates return by hidden pointer in %rdi; rmc's builtins do
            // not, so this only appears for functions this backend owns.
            return Err("mir_codegen: aggregate return unsupported".into());
        }
        if !matches!(dest.proj.as_slice(), []) || self.local_ty(dest.local) != Ty::Unit {
            // A `()` destination still has a slot; storing is harmless.
            self.store_scalar(dest, "%rax")?;
        }
        Ok(())
    }

    fn operand_ty(&self, op: &Operand) -> Ty {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.place_ty(p),
            Operand::Const(Const::Bool(_)) => Ty::Bool,
            Operand::Const(Const::Str(_)) => Ty::Ptr(Box::new(Ty::Unit)),
            Operand::Const(_) => Ty::I64,
        }
    }
}

fn align_up(v: i64, a: i64) -> i64 {
    (v + a - 1) / a * a
}

fn ty_size(ty: &Ty) -> i64 {
    match ty {
        Ty::I64 | Ty::Bool | Ty::Int | Ty::Ptr(_) | Ty::Ref(_) | Ty::Other | Ty::Unit => 8,
        Ty::Struct(name) => 8, // refined by the aggregate's own length
        Ty::Array(elem, n) => ty_size(elem) * (*n as i64).max(1),
        Ty::Tuple(tys) => tys.iter().map(|t| ty_size(t)).sum::<i64>().max(8),
        _ => 8,
        #[allow(unreachable_patterns)]
        _ => 8,
    }
}

fn is_compare(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

fn jcc(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "je",
        BinOp::Ne => "jne",
        BinOp::Lt => "jl",
        BinOp::Le => "jle",
        BinOp::Gt => "jg",
        BinOp::Ge => "jge",
        _ => "je",
    }
}

/// The local whose value is the discriminant of a `switchInt(Lt(..))`.
fn operand_place(op: &Operand) -> Option<&Place> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => Some(p),
        _ => None,
    }
}

fn setcc(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "sete",
        BinOp::Ne => "setne",
        BinOp::Lt => "setl",
        BinOp::Le => "setle",
        BinOp::Gt => "setg",
        BinOp::Ge => "setge",
        _ => "sete",
    }
}

/// Count how often each local is read, and note which locals are projected or
/// have their address taken (both disqualify promotion).
fn scan_rv(
    rv: &Rvalue,
    counts: &mut HashMap<u32, u32>,
    projected: &mut HashSet<u32>,
    addr_taken: &mut HashSet<u32>,
) {
    match rv {
        Rvalue::Use(op) => scan_operand(op, counts, projected),
        Rvalue::Binary(_, a, b) => {
            scan_operand(a, counts, projected);
            scan_operand(b, counts, projected);
        }
        Rvalue::Unary(_, a) => scan_operand(a, counts, projected),
        Rvalue::Cast { op, .. } => scan_operand(op, counts, projected),
        Rvalue::Repeat { op, .. } => scan_operand(op, counts, projected),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => {
            for op in ops {
                scan_operand(op, counts, projected);
            }
        }
        Rvalue::Ref { place, .. } | Rvalue::RawRef { place, .. } => {
            bump(place, counts, projected);
            addr_taken.insert(place.local);
        }
        Rvalue::Len(p) => {
            bump(p, counts, projected);
            addr_taken.insert(p.local);
        }
        Rvalue::Unsupported(_) => {}
    }
}

fn scan_operand(op: &Operand, counts: &mut HashMap<u32, u32>, projected: &mut HashSet<u32>) {
    if let Operand::Copy(p) | Operand::Move(p) = op {
        bump(p, counts, projected);
    }
}

fn scan_term(
    term: &Terminator,
    counts: &mut HashMap<u32, u32>,
    projected: &mut HashSet<u32>,
    _addr_taken: &mut HashSet<u32>,
) {
    match term {
        Terminator::SwitchInt { discr, .. } => scan_operand(discr, counts, projected),
        Terminator::Call { dest, args, .. } => {
            bump(dest, counts, projected);
            for a in args {
                scan_operand(a, counts, projected);
            }
        }
        _ => {}
    }
}

fn bump(p: &Place, counts: &mut HashMap<u32, u32>, projected: &mut HashSet<u32>) {
    *counts.entry(p.local).or_insert(0) += 1;
    if !p.proj.is_empty() {
        projected.insert(p.local);
    }
}

