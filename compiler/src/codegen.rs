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
                    Expr::Index(base, idx) => {
                        match Self::array_elem_operand(base, idx) {
                            // Constant index: one store.
                            Some(op) if !op.contains("%rax") => {
                                self.gen_expr(value);
                                self.line(&format!("mov %rax, {op}"));
                            }
                            // Variable index: load the index into %rax, then
                            // store through `disp(%rbp,%rax,8)`.
                            Some(op) => {
                                match Self::simple_operand(value) {
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
                match Self::array_elem_operand(base, _idx) {
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

    /// An operand usable directly in an instruction without touching any
    /// register: an immediate, a scalar local, or an array element with a
    /// constant index. Anything needing a register (a variable index) returns
    /// `None` so the caller falls back to materialising the value in `%rax`.
    fn simple_operand(e: &Expr) -> Option<String> {
        match e {
            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => Some(format!("${v}")),
            Expr::Bool(b) => Some(format!("${}", if *b { 1 } else { 0 })),
            Expr::Var { slot, ty, .. } => match ty {
                Type::Array(..) => None,
                _ => Some(format!("{}(%rbp)", Self::off(*slot))),
            },
            Expr::Index(base, idx) => {
                match Self::array_elem_operand(base, idx) {
                    // Only register-free operands are usable here.
                    Some(op) if !op.contains("%rax") => Some(op),
                    _ => None,
                }
            }
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

        // Division/modulo: the dividend must be in `%rax` and the divisor in a
        // register, so there is no operand-folding shortcut.
        if matches!(op, Div | Mod) {
            self.eval_operands(a, b);
            self.line("cqto");
            self.line("idiv %rcx");
            if matches!(op, Mod) {
                self.line("mov %rdx, %rax");
            }
            return Type::I64;
        }

        // Add/Sub/Mul: keep the left operand in `%rax` and the right in `%rcx`.
        // Folding a memory operand straight into `imul`/`add` measured slower on
        // the cores this was tuned on, so the plain two-register form is used.
        self.eval_operands(a, b);
        match op {
            Add => self.line("add %rcx, %rax"),
            Sub => self.line("sub %rcx, %rax"),
            Mul => self.line("imul %rcx, %rax"),
            _ => unreachable!(),
        }
        Type::I64
    }

    /// An operand `disp(%rbp,%rax,8)` for `base[idx]` where `base` is a local
    /// array. The caller must have the index value in `%rax`. Returns `None`
    /// for anything else, so the caller falls back to computing the address.
    fn array_elem_operand(base: &Expr, idx: &Expr) -> Option<String> {
        let (slot, n) = match base {
            Expr::Var { slot, ty: Type::Array(_, n), .. } => (*slot, *n),
            _ => return None,
        };
        match idx {
            Expr::Int(v) if (-2_147_483_648..=2_147_483_647).contains(v) => {
                // Constant index: no register needed at all.
                Some(format!("{}(%rbp)", Self::array_off(slot, n) + *v as i32 * 8))
            }
            Expr::Var { slot: _, ty: Type::I64, .. } => {
                // The index lives in a slot; the caller must load it into %rax
                // and bake the array base offset into the displacement.
                Some(format!("{}(%rbp,%rax,8)", Self::array_off(slot, n)))
            }
            _ => None,
        }
    }

    /// Emit a `cmp` that leaves the flags for `a op b` (i.e. `a - b`). Returns
    /// false when neither operand can be named without a register. Immediate
    /// and memory operands get an explicit `q` suffix where x86 requires it,
    /// and two-memory or two-immediate forms fall back to a register.
    fn emit_cmp(&mut self, a: &Expr, b: &Expr) -> bool {
        let av = Self::simple_operand(a);
        let bv = Self::simple_operand(b);
        match (av, bv) {
            (Some(x), Some(y)) => {
                let xi = x.starts_with('$');
                let yi = y.starts_with('$');
                if xi && yi {
                    self.line(&format!("mov {x}, %rax"));
                    self.line(&format!("cmp {y}, %rax"));
                } else if xi {
                    self.line(&format!("cmpq {x}, {y}"));
                } else if yi {
                    self.line(&format!("cmpq {y}, {x}"));
                } else {
                    self.gen_expr(b);
                    self.line(&format!("cmp %rax, {x}"));
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

    fn gen_call(&mut self, name: &str, args: &[Expr]) -> Type {
        const ARG_REGS: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
        // Simple arguments evaluate with a single load that touches neither
        // `%rcx` nor `%rdx`, so they can go straight into the argument
        // registers for any arity. For one or two arguments the registers used
        // are `%rdi`/`%rsi`, which expression evaluation never clobbers, so any
        // argument without a nested call is safe too.
        let direct = args.iter().all(Self::arg_is_simple)
            || (args.len() <= 2 && !args.iter().any(Self::contains_call));
        if direct {
            for (a, r) in args.iter().zip(ARG_REGS.iter()) {
                self.gen_expr(a);
                self.line(&format!("mov %rax, {r}"));
            }
        } else {
            // Evaluate arguments right-to-left and push them, so that after
            // popping they land in the ABI registers in the correct order.
            for a in args.iter().rev() {
                self.gen_expr(a);
                self.push_rax();
            }
            for r in ARG_REGS.iter().take(args.len()) {
                self.pop_to(r);
            }
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

    /// An argument that loads into a register without using the `%rcx`/`%rdx`
    /// scratch registers that later arguments would clobber.
    fn arg_is_simple(e: &Expr) -> bool {
        Self::simple_operand(e).is_some() || matches!(e, Expr::Str(_))
    }

    /// True when evaluating `e` may perform a call.
    fn contains_call(e: &Expr) -> bool {
        match e {
            Expr::Call(..) => true,
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
