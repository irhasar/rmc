//! Direct x86-64 (System V, AT&T syntax) code generator.
//!
//! Expression results live in `%rax`. Temporaries are pushed on the machine
//! stack, which keeps the generator simple and register allocation unnecessary.
//! Every local variable occupies an 8-byte slot in the frame at
//! `-(slot+1)*8(%rbp)`, growing downward from the saved frame pointer.

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
}

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
        }
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

    /// Byte offset of stack `slot` relative to `%rbp`. Slots grow downward so
    /// they never overlap the saved `%rbp` and return address above it.
    fn off(slot: i32) -> i32 {
        -(slot + 1) * 8
    }

    /// Address of element 0 of an `n`-element array occupying `slot..slot+n`.
    /// Slots are laid out downward, so the lowest address is that of the last
    /// slot; elements are then addressed upward from there as `base + i*8`.
    fn array_off(slot: i32, n: usize) -> i32 {
        -(slot + n as i32) * 8
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
        self.raw(&format!(".globl {}", f.name));
        self.raw(&format!(".type {}, @function", f.name));
        self.raw(&format!("{}:", f.name));
        self.line("push %rbp");
        self.line("mov %rsp, %rbp");
        let frame = (((f.nslots * 8) + 15) / 16) * 16;
        if frame > 0 {
            self.line(&format!("sub ${frame}, %rsp"));
        }
        // Spill incoming integer arguments into their slots.
        const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
        for (i, _) in f.params.iter().enumerate() {
            if i < 6 {
                self.line(&format!("mov {}, {}(%rbp)", ARG_REGS[i], Self::off(i as i32)));
            }
        }
        let ty = self.gen_block(&f.body);
        if ty == Type::Unit {
            self.line("xor %eax, %eax");
        }
        self.raw(&format!("{}:", self.epilogue));
        self.line("leave");
        self.line("ret");
        self.raw(&format!(".size {}, .-{}", f.name, f.name));
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
                    let value_src = match init {
                        Some(Expr::ArrayLit(v, _)) => Some(v.as_ref()),
                        _ => None,
                    };
                    match value_src {
                        Some(v) => {
                            self.gen_expr(v);
                        }
                        None => self.line("xor %eax, %eax"),
                    }
                    self.fill_array(*slot, elem.size(), *n);
                } else if let Some(e) = init {
                    self.gen_expr(e);
                    self.line(&format!("mov %rax, {}(%rbp)", Self::off(*slot)));
                } else {
                    self.line(&format!("movq $0, {}(%rbp)", Self::off(*slot)));
                }
            }
            Stmt::Assign { target, value } => {
                match target {
                    Expr::Var { slot, .. } => {
                        self.gen_expr(value);
                        self.line(&format!("mov %rax, {}(%rbp)", Self::off(*slot)));
                    }
                    Expr::Index(..) => {
                        self.gen_addr(target);
                        self.push_rax();
                        self.gen_expr(value);
                        self.pop_to("%rcx");
                        self.line("mov %rax, (%rcx)");
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
            Stmt::Return(v) => {
                match v {
                    Some(e) => {
                        self.gen_expr(e);
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

    /// Zero-free store of `%rax` into `n` consecutive slots starting at `slot`.
    fn fill_array(&mut self, slot: i32, elem_size: usize, n: usize) {
        if elem_size != 8 {
            // Only 8-byte elements are supported by sema.
            return;
        }
        let base = Self::array_off(slot, n);
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

    /// Leaves the address of an lvalue in `%rax`.
    fn gen_addr(&mut self, e: &Expr) {
        match e {
            Expr::Var { slot, ty, .. } => {
                let off = match ty {
                    Type::Array(_, n) => Self::array_off(*slot, *n),
                    _ => Self::off(*slot),
                };
                self.line(&format!("lea {off}(%rbp), %rax"));
            }
            Expr::Index(base, idx) => {
                self.gen_addr(base);
                self.push_rax();
                self.gen_expr(idx);
                self.pop_to("%rcx");
                self.line("lea (%rcx,%rax,8), %rax");
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
                match ty {
                    Type::Array(_, n) => {
                        self.line(&format!("lea {}(%rbp), %rax", Self::array_off(*slot, *n)));
                    }
                    _ => {
                        self.line(&format!("mov {}(%rbp), %rax", Self::off(*slot)));
                    }
                }
                ty.clone()
            }
            Expr::Index(base, _idx) => {
                let elem = match base.as_ref() {
                    Expr::Var { ty: Type::Array(e, _), .. } => (**e).clone(),
                    _ => Type::I64,
                };
                self.gen_addr(e);
                self.line("mov (%rax), %rax");
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
            Expr::Call(name, args) => self.gen_call(name, args),
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

    /// Recognise an operand that can be used directly as an x86 memory or
    /// immediate operand, avoiding a load into `%rax` and a push/pop pair.
    fn simple_operand(e: &Expr) -> Option<String> {
        match e {
            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => Some(format!("${v}")),
            Expr::Bool(b) => Some(format!("${}", if *b { 1 } else { 0 })),
            Expr::Var { slot, ty, .. } => match ty {
                Type::Array(..) => None,
                _ => Some(format!("{}(%rbp)", Self::off(*slot))),
            },
            _ => None,
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

        // Fold constant and local operands into the instructions as immediate
        // or memory operands instead of routing them through the stack.
        self.eval_operands(a, b);

        match op {
            Add => self.line("add %rcx, %rax"),
            Sub => self.line("sub %rcx, %rax"),
            Mul => self.line("imul %rcx, %rax"),
            Div => {
                self.line("cqto");
                self.line("idiv %rcx");
            }
            Mod => {
                self.line("cqto");
                self.line("idiv %rcx");
                self.line("mov %rdx, %rax");
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                self.line("cmp %rcx, %rax");
                self.line(&format!("{} %al", setcc(op)));
                self.line("movzbq %al, %rax");
                return Type::Bool;
            }
            And | Or => unreachable!(),
        }
        match op {
            Eq | Ne | Lt | Le | Gt | Ge => Type::Bool,
            _ => Type::I64,
        }
    }

    /// Evaluate two operands into the shared convention: left in `%rax`, right
    /// in `%rcx`. Constants and locals are folded in as immediate or memory
    /// operands; anything else goes through the stack.
    fn eval_operands(&mut self, a: &Expr, b: &Expr) {
        let ra = Self::simple_operand(a);
        let rb = Self::simple_operand(b);
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
                self.eval_operands(a, b);
                self.line("cmp %rcx, %rax");
                self.line(&format!("{cc} {target}"));
                return;
            }
        }
        // `!(a < b)` jumps when the comparison holds.
        if let Expr::Un(UnOp::Not, inner) = cond {
            if let Expr::Bin(op, a, b) = inner.as_ref() {
                if let Some(cc) = setcc_cc(*op) {
                    self.eval_operands(a, b);
                    self.line("cmp %rcx, %rax");
                    self.line(&format!("{cc} {target}"));
                    return;
                }
            }
        }
        self.gen_expr(cond);
        self.line("test %rax, %rax");
        self.line(&format!("jz {target}"));
    }

    fn gen_call(&mut self, name: &str, args: &[Expr]) -> Type {
        // Evaluate arguments right-to-left and push them, so that after popping
        // they land in the ABI registers in the correct order.
        for a in args.iter().rev() {
            self.gen_expr(a);
            self.push_rax();
        }
        const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
        for r in ARG_REGS.iter().take(args.len()) {
            self.pop_to(r);
        }
        let pad = self.sp_off % 2 != 0;
        if pad {
            self.line("sub $8, %rsp");
        }
        self.line(&format!("call {name}"));
        if pad {
            self.line("add $8, %rsp");
        }
        // Return type is resolved by sema; nrc runtime helpers return i64.
        Type::I64
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
