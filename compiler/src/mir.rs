//! Parser and in-memory model for the MIR text that `rustc --emit=mir` prints.
//!
//! This module is the *front end adapter* of the `rmc` backend: rustc does the
//! lexing, parsing, name resolution, type checking and MIR-level optimisation,
//! and hands back a small, fully typed control-flow graph. The backend
//! (`mir_codegen.rs`) then lowers that graph to x86-64, reusing `elf.rs` and
//! `runtime.rs` unchanged.
//!
//! The parser reads the stable-ish "human consumers" text format. It is
//! deliberately permissive about what it ignores: `debug` bindings, `scope`
//! trees, `StorageLive`/`StorageDead`, `assert`, `unwind` edges and the
//! `inlined` scope annotations carry no semantics this backend needs, so they
//! are skipped rather than modelled. Anything it does not recognise is a hard
//! error naming the line, so an unhandled MIR shape fails loudly instead of
//! silently miscompiling.

use std::collections::{HashMap, HashSet};

/// A resolved type, in the narrow sense the backend needs.
///
/// Every type is either a scalar (lives in one 64-bit register) or an aggregate
/// (lives in memory, named by its address). `Ref`/`RawPtr` are scalars: a
/// pointer is one word, and a dereference reads through it.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    I64,
    Bool,
    /// `usize`/`isize`/`u*`/`i*` narrower than 64 bits and `u64`/`i64` all live
    /// in one 64-bit word here; the width only matters to the exact op, and
    /// every benchmark program is `i64`-shaped.
    Int,
    Ptr(TyBox),
    /// A reference or raw pointer; the pointee type is only used to size a
    /// `&raw` / `*p` load.
    Ref(TyBox),
    Struct(String),
    Array(TyBox, u64),
    Tuple(Vec<TyBox>),
    /// `()` and any type the backend treats as zero-sized.
    Unit,
    /// A `&str` literal or an unmodelled type; treated as a scalar.
    Other,
}

pub type TyBox = Box<Ty>;

impl Ty {
    pub fn is_aggregate(&self) -> bool {
        matches!(self, Ty::Struct(_) | Ty::Array(..) | Ty::Tuple(_))
    }
}

/// One operand of a statement or terminator.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Const(Const),
}

/// A constant operand.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(i64),
    Bool(bool),
    Unit,
    Str(String),
    /// `{allocN: TY}` — the address of a static allocation (a promoted
    /// constant, a string, or a `static`'s data). `place` is the optional
    /// `[place]` offset the printer appends when the constant is an interior
    /// pointer.
    Alloc {
        id: u32,
        place: Option<Box<Place>>,
    },
    /// A named `alloc` (string) or a `const` item reference we do not fold.
    Named(String),
}

/// A memory location: a base local plus a path of projections.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub local: u32,
    pub proj: Vec<Proj>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Proj {
    /// `.field` — a tuple index or a struct field index (MIR prints both as a
    /// number; `(x.0)`, `(x.1)`).
    Field(u32),
    /// `[i]` with a constant index, from `x[3 of 4]` or `x[3]`.
    IndexConst(u64),
    /// `[*i]` with a local index.
    IndexLocal(u32),
    /// `*p` — dereference the pointer held in this place.
    Deref,
    /// `&raw const ...` / `&raw mut ...` — the *address* of the place rather
    /// than its value. Consumed by an `AddrOf` rvalue.
    RawRef,
}

/// The right-hand side of an assignment.
#[derive(Debug, Clone)]
pub enum Rvalue {
    Use(Operand),
    Binary(BinOp, Operand, Operand),
    Unary(UnOp, Operand),
    /// `a as T (kind)`; `from`/`to` are the source and destination types.
    Cast { from: TyBox, to: TyBox, op: Operand, kind: CastKind },
    /// `&place` or `&mut place`.
    Ref { mutbl: bool, place: Place },
    /// `&raw const place` / `&raw mut place`.
    RawRef { mutbl: bool, place: Place },
    /// `[value; count]` — a repeat array literal.
    Repeat { op: Operand, count: u64 },
    /// `[a, b, c]` — a list array literal.
    Array(Vec<Operand>),
    /// `Struct { f: v, .. }` — a struct/tuple literal. Field names are printed
    /// in declaration order, which is the layout order.
    Aggregate(Vec<Operand>),
    /// `Len(place)` — the array length of a place.
    Len(Place),
    /// `AddWithOverflow(a, b)` — a wrapping `a OP b` whose result and overflow
    /// flag are stored as `(result, flag)` into the destination aggregate.
    WithOverflow { op: BinOp, a: Operand, b: Operand },
    /// `discriminant(place)` — the tag of an enum place.
    Discriminant(Place),
    /// A value the backend does not model (a constant expression, a vtable
    /// pointer, ...). Treated as 0 with a diagnostic.
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Rem,
    BitXor, BitAnd, BitOr, Shl, Shr,
    Eq, Ne, Lt, Le, Gt, Ge,
    Cmp, Offset, PtrOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    PtrMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastKind {
    IntToInt,
    PtrToPtr,
    IntToPtr,
    PtrToInt,
    PtrToIntCast,
    IntToPtrCast,
    Other,
}

/// A statement inside a basic block.
#[derive(Debug, Clone)]
pub enum Stmt {
    Assign(Place, Rvalue),
    /// A `StorageLive`/`StorageDead`/`Nop`/`FakeRead`/... — no code emitted.
    Nop,
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Goto(u32),
    SwitchInt {
        discr: Operand,
        targets: Vec<(i64, u32)>,
        otherwise: u32,
    },
    Return,
    Call {
        dest: Place,
        func: CallTarget,
        args: Vec<Operand>,
        target: Option<u32>,
    },
    Unreachable,
    /// `assert(cond) -> [success: bbX, unwind ...]`: continue to `target` when
    /// the condition holds, otherwise trap (exit 101).
    Assert {
        cond: Operand,
        negate: bool,
        target: u32,
    },
    /// A trap (e.g. `assert` failure with `unwind terminate`); the backend
    /// emits an `exit(101)`.
    Abort,
}

#[derive(Debug, Clone)]
pub enum CallTarget {
    /// A direct call to a named function (which may be a runtime builtin).
    Direct(String),
    /// An indirect call through a local (a function pointer); unsupported.
    Indirect,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub stmts: Vec<Stmt>,
    pub term: Terminator,
}

#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub ty: TyBox,
    pub is_arg: bool,
    /// The source name, when the local is a `debug x => _N;` binding.
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<BasicBlock>,
    /// `const NAME: TY = VALUE;` items declared at this function's scope.
    pub consts: HashMap<String, i64>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub funcs: Vec<Function>,
    /// `allocN` static data blobs (string literals, promoted constants,
    /// `static` data), by allocation id.
    pub allocs: HashMap<u32, AllocData>,
    /// `const NAME: TY = ...;` items at the crate scope.
    pub consts: HashMap<String, i64>,
}

/// A static allocation: its bytes, any pointer relocations into other
/// allocations, and the `static` name it backs (when it is one).
#[derive(Debug, Clone, Default)]
pub struct AllocData {
    pub bytes: Vec<u8>,
    /// `(byte offset, target alloc id)` for each interior pointer.
    pub relocs: Vec<(usize, u32)>,
    /// The `static NAME` this allocation initialises, if any.
    pub static_name: Option<String>,
}

impl Program {
    pub fn func(&self, name: &str) -> Option<&Function> {
        self.funcs.iter().find(|f| f.name == name)
    }

    /// Copy-propagate the whole crate.
    pub fn copy_propagate(&mut self) {
        for f in &mut self.funcs {
            f.copy_propagate();
        }
    }
}

impl Function {
    /// Coalesce `_d = copy _x` temporaries: within a block, rewrite later uses
    /// of `_d` to read `_x` directly.
    ///
    /// rustc's MIR routes almost every value through a fresh temporary
    /// (`_6 = copy _4; _5 = Lt(move _6, move _7)`), so without this pass each
    /// operand costs a store to a frame slot and a reload, and the register
    /// allocator spends its budget on single-use temporaries instead of the
    /// variables that live across the loop. Substituting the source local keeps
    /// the value in one place instead of two.
    ///
    /// Only single-definition, non-argument locals are substituted (a
    /// rustc temporary), and the alias is dropped at any write to the source,
    /// so a source that is reassigned later in the block is never read stale.
    /// The alias is block-local, so a use in another block still reads the
    /// temporary's slot, which is still written.
    fn copy_propagate(&mut self) {
        let defs = self.def_counts();
        let subst_local: HashSet<u32> = (0..self.locals.len() as u32)
            .filter(|l| self.safe_subst(*l, &defs))
            .collect();
        // Locals written through a projection anywhere (`_s.0 = x`, `(*_s).1 = y`).
        // A value read out of such a local can change without a whole-local
        // write, so aliasing it is unsafe.
        let mutated: HashSet<u32> = self
            .blocks
            .iter()
            .flat_map(|b| b.stmts.iter())
            .filter_map(|s| match s {
                Stmt::Assign(d, _) if !d.proj.is_empty() => Some(d.local),
                _ => None,
            })
            .collect();
        for b in &mut self.blocks {
            let mut subst: HashMap<u32, Place> = HashMap::new();
            let mut kept: Vec<Stmt> = Vec::with_capacity(b.stmts.len());
            for mut s in b.stmts.drain(..) {
                rewrite_stmt(&mut s, &subst);
                if let Some(l) = written_local(&s) {
                    // A write to `l` invalidates every alias that reads it, and
                    // every alias that reads through it (a projection of `l`).
                    subst.retain(|k, v| *k != l && v.local != l);
                }
                if let Stmt::Assign(d, Rvalue::Use(Operand::Copy(src) | Operand::Move(src))) = &s {
                    if d.proj.is_empty()
                        && subst_local.contains(&d.local)
                        && place_aliasable(src, &mutated)
                    {
                        subst.insert(d.local, src.clone());
                    }
                }
                if !is_self_assign(&s) {
                    kept.push(s);
                }
            }
            b.stmts = kept;
            rewrite_term(&mut b.term, &subst);
        }
        self.drop_dead_copies();
    }

    /// Drop `_d = copy _s` when nothing reads `_d` any more.
    ///
    /// Copy propagation rewrites every use of the temporary away, which leaves
    /// the copy itself dead. It is a plain move of one scalar, so removing it
    /// cannot drop work: only a `Use` of a scalar whole-local is removed, never
    /// a call, an aggregate copy or a trapping arithmetic op.
    fn drop_dead_copies(&mut self) {
        let uses = self.use_counts();
        let live: HashSet<u32> = (0..self.locals.len() as u32)
            .filter(|l| {
                if self.local_ty(*l).is_aggregate() {
                    return true;
                }
                if self.locals.get(*l as usize).map(|x| x.is_arg).unwrap_or(true) {
                    return true;
                }
                uses.get(l).copied().unwrap_or(0) > 0
            })
            .collect();
        for b in &mut self.blocks {
            b.stmts.retain(|s| {
                let Stmt::Assign(d, Rvalue::Use(_)) = s else { return true };
                !d.proj.is_empty() || d.local == 0 || live.contains(&d.local)
            });
        }
    }

    /// The number of reads of each local, over the (already rewritten) body.
    fn use_counts(&self) -> HashMap<u32, u32> {
        let mut uses: HashMap<u32, u32> = HashMap::new();
        let mut read_op = |op: &Operand, uses: &mut HashMap<u32, u32>| {
            if let Operand::Copy(p) | Operand::Move(p) = op {
                *uses.entry(p.local).or_insert(0) += 1;
            }
        };
        for b in &self.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    scan_rv_operands(rv, &mut |op| read_op(op, &mut uses));
                }
            }
            match &b.term {
                Terminator::SwitchInt { discr, .. } => read_op(discr, &mut uses),
                Terminator::Assert { cond, .. } => read_op(cond, &mut uses),
                Terminator::Call { args, .. } => {
                    for a in args {
                        read_op(a, &mut uses);
                    }
                }
                _ => {}
            }
        }
        uses
    }

    /// The number of statements that assign each local (including calls).
    fn def_counts(&self) -> HashMap<u32, u32> {
        let mut defs: HashMap<u32, u32> = HashMap::new();
        for b in &self.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(p, _) = s {
                    *defs.entry(p.local).or_insert(0) += 1;
                }
            }
            if let Terminator::Call { dest, .. } = &b.term {
                *defs.entry(dest.local).or_insert(0) += 1;
            }
        }
        defs
    }

    /// A destination local is safe to substitute when it is written exactly
    /// once (so it is a single-use temporary rather than a live variable that
    /// could be observed while holding its old value) and is not an argument.
    fn safe_subst(&self, local: u32, defs: &HashMap<u32, u32>) -> bool {
        if self.locals.get(local as usize).map(|l| l.is_arg).unwrap_or(true) {
            return false;
        }
        defs.get(&local).copied().unwrap_or(0) == 1
    }

    fn local_ty(&self, local: u32) -> Ty {
        self.locals
            .get(local as usize)
            .map(|l| (*l.ty).clone())
            .unwrap_or(Ty::I64)
    }
}

/// Call `f` for every operand a statement reads.
fn scan_rv_operands(rv: &Rvalue, f: &mut impl FnMut(&Operand)) {
    match rv {
        Rvalue::Use(op) => f(op),
        Rvalue::Binary(_, a, b) => {
            f(a);
            f(b);
        }
        Rvalue::Unary(_, a) => f(a),
        Rvalue::Cast { op, .. } => f(op),
        Rvalue::Repeat { op, .. } => f(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => ops.iter().for_each(|op| f(op)),
        Rvalue::WithOverflow { a, b, .. } => {
            f(a);
            f(b);
        }
        // These read a place rather than an operand: the place's base local is
        // still a read (its address is taken or its length/tag inspected), and
        // any `[i]` index is read too.
        Rvalue::Ref { place, .. }
        | Rvalue::RawRef { place, .. }
        | Rvalue::Len(place)
        | Rvalue::Discriminant(place) => {
            f(&Operand::Copy(place.clone()));
            for proj in &place.proj {
                if let Proj::IndexLocal(i) = proj {
                    f(&Operand::Copy(Place { local: *i, proj: Vec::new() }));
                }
            }
        }
        _ => {}
    }
}

fn is_self_assign(s: &Stmt) -> bool {
    let Stmt::Assign(d, Rvalue::Use(Operand::Copy(src) | Operand::Move(src))) = s else {
        return false;
    };
    d == src
}

/// The local a statement writes, if it writes a whole local.
fn written_local(s: &Stmt) -> Option<u32> {
    match s {
        Stmt::Assign(d, _) if d.proj.is_empty() => Some(d.local),
        _ => None,
    }
}

/// Whether `p` can stand in for another place in an alias.
///
/// Only a plain local or a projection that does not read through a pointer is
/// aliased. A projection rooted at a `Deref`/`RawRef` (e.g. `(*_5).0`) is
/// skipped: a later store through the same pointer would invalidate the value,
/// and the cheap invalidation here only watches whole-local writes.
fn place_aliasable(p: &Place, mutated: &HashSet<u32>) -> bool {
    if mutated.contains(&p.local) {
        return false;
    }
    p.proj.iter().all(|proj| {
        !matches!(proj, Proj::Deref | Proj::RawRef)
    })
}

fn rewrite_op(op: &mut Operand, subst: &HashMap<u32, Place>) {
    if let Operand::Copy(p) | Operand::Move(p) = op {
        if p.proj.is_empty() {
            if let Some(src) = subst.get(&p.local) {
                *p = src.clone();
            }
        }
    }
}

/// Rewrite every place a statement reads through the alias map. A `Ref`/`RawRef`
/// target is rewritten too, but only when the alias is a plain local: aliasing a
/// projection would change which address is taken.
fn rewrite_place(p: &mut Place, subst: &HashMap<u32, Place>) {
    if p.proj.is_empty() {
        if let Some(src) = subst.get(&p.local) {
            *p = src.clone();
        }
    }
}

fn rewrite_stmt(s: &mut Stmt, subst: &HashMap<u32, Place>) {
    if subst.is_empty() {
        return;
    }
    let Stmt::Assign(_, rv) = s else { return };
    match rv {
        Rvalue::Use(op) => rewrite_op(op, subst),
        Rvalue::Binary(_, a, b) => {
            rewrite_op(a, subst);
            rewrite_op(b, subst);
        }
        Rvalue::Unary(_, a) => rewrite_op(a, subst),
        Rvalue::Cast { op, .. } => rewrite_op(op, subst),
        Rvalue::Repeat { op, .. } => rewrite_op(op, subst),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => {
            for op in ops {
                rewrite_op(op, subst);
            }
        }
        Rvalue::WithOverflow { a, b, .. } => {
            rewrite_op(a, subst);
            rewrite_op(b, subst);
        }
        // `Len`/`Discriminant` read a type-determined property of a whole
        // local, which a value copy preserves, so the place can be substituted.
        // `Ref`/`RawRef` deliberately are not rewritten: `_d = copy _s` gives
        // `_d` its own storage, so `&_d` is not `&_s`.
        Rvalue::Len(place) | Rvalue::Discriminant(place) => {
            if place.proj.is_empty() {
                rewrite_place(place, subst);
            }
        }
        _ => {}
    }
}

fn rewrite_term(t: &mut Terminator, subst: &HashMap<u32, Place>) {
    if subst.is_empty() {
        return;
    }
    match t {
        Terminator::SwitchInt { discr, .. } => rewrite_op(discr, subst),
        Terminator::Assert { cond, .. } => rewrite_op(cond, subst),
        Terminator::Call { args, .. } => {
            for a in args {
                rewrite_op(a, subst);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse the text `rustc --emit=mir` prints into a [`Program`].
pub fn parse(text: &str) -> Result<Program, String> {
    let lines: Vec<String> = text.lines().map(strip_comment).collect();
    let mut prog = Program { funcs: Vec::new(), allocs: HashMap::new(), consts: HashMap::new() };
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if l.is_empty() {
            i += 1;
            continue;
        }
        if l.starts_with("fn ") {
            let (f, next) = parse_fn(&lines, i)?;
            prog.funcs.push(f);
            i = next;
        } else if l.starts_with("const ") {
            if let Some((name, val)) = parse_const_item(l) {
                prog.consts.insert(name, val);
            }
            i += 1;
        } else if l.starts_with("alloc") && l.contains("size:") {
            let (id, data, next) = parse_alloc(&lines, i)?;
            prog.allocs.insert(id, data);
            i = next;
        } else {
            i += 1;
        }
    }
    Ok(prog)
}

fn strip_comment(line: &str) -> String {
    match line.find("//") {
        Some(i) => line[..i].to_string(),
        None => line.to_string(),
    }
}

/// `const NAME: TY = VALUE;` — returns the integer value when it has one.
fn parse_const_item(line: &str) -> Option<(String, i64)> {
    let t = line.trim().trim_end_matches(';');
    let t = t.strip_prefix("const ")?;
    let colon = t.find(':')?;
    let name = t[..colon].trim().to_string();
    let eq = t.find('=')?;
    let val = t[eq + 1..].trim();
    let v = match parse_const(val) {
        Some(Const::Int(n)) => n,
        Some(Const::Bool(b)) => b as i64,
        _ => return None,
    };
    Some((name, v))
}

/// `allocN (size: S, align: A) {` followed by hex rows until `}`.
///
/// A row may also carry a pointer relocation, printed as a run of box-drawing
/// characters, then `allocK<imm>`, then the offset's little-endian bytes and a
/// `│`-separated preview. The relocation is recorded as `(offset, allocK)` and
/// its bytes are skipped.
fn parse_alloc(lines: &[String], start: usize) -> Result<(u32, AllocData, usize), String> {
    let header = lines[start].trim();
    let id: u32 = header
        .strip_prefix("alloc")
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("mir: bad alloc header `{header}`"))?;
    let static_name = header
        .split_once("static:")
        .and_then(|(_, r)| r.split(',').next())
        .map(|n| n.trim().to_string());
    let mut data = AllocData { bytes: Vec::new(), relocs: Vec::new(), static_name };
    let mut i = start + 1;
    while i < lines.len() {
        let l = lines[i].trim();
        if l.starts_with('}') {
            return Ok((id, data, i + 1));
        }
        // The dump is a table split by the box-drawing `│`. Two shapes appear:
        //   `╾──alloc1<imm>──╼ 0f 00 .. │ ascii`   (data, then a preview)
        //   `0x00 │ 0e 00 .. │ ascii`              (offset, data, preview)
        // so the data field is the first one unless the line opens with a
        // `0x..` offset column, in which case it is the second.
        let fields: Vec<&str> = l.split('│').collect();
        let data_field = if fields[0].trim().starts_with("0x") && fields.len() >= 2 {
            fields[1]
        } else {
            fields[0]
        };
        for tok in data_field.split_whitespace() {
            // A `__` marks padding past the initialised bytes, not a byte.
            if tok == "__" {
                continue;
            }
            if tok.contains("alloc") && tok.contains("<imm>") {
                let Some(pos) = tok.find("alloc") else { continue };
                let Some(target) = tok[pos + 5..]
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                else {
                    continue;
                };
                let off = data.bytes.len();
                data.relocs.push((off, target));
                data.bytes.extend_from_slice(&[0u8; 8]);
            } else if let Some(hex) = tok.strip_prefix("0x") {
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    data.bytes.push(b);
                }
            } else if let Ok(b) = u8::from_str_radix(tok, 16) {
                data.bytes.push(b);
            }
        }
        i += 1;
    }
    Err("mir: unterminated alloc block".into())
}

fn parse_fn(lines: &[String], start: usize) -> Result<(Function, usize), String> {
    let header = lines[start].trim();
    let rest = header.strip_prefix("fn ").unwrap();
    let open = rest.find('(').ok_or_else(|| format!("mir: bad fn header `{header}`"))?;
    let name = rest[..open].trim().to_string();
    // The emitted symbol is the bare definition name, but a call prints the
    // qualified path. Map `...::name` (and `<T as Tr>::name`) back to `name` so
    // direct calls resolve to the definition.
    let name = normalize_call_name(&name).unwrap_or(name);
    let close = matching(rest, open, '(', ')').ok_or_else(|| format!("mir: unbalanced fn args `{header}`"))?;
    let arg_tys = parse_arg_tys(&rest[open + 1..close])?;
    let after = rest[close + 1..].trim();
    let _ret = match after.strip_prefix("->") {
        Some(r) => parse_ty(r.trim().trim_end_matches('{').trim())?,
        None => Ty::Unit,
    };

    // Body runs to the first column-0 `}`.
    let mut end = start + 1;
    while end < lines.len() {
        let raw = &lines[end];
        if !raw.is_empty() && raw.as_bytes()[0] != b' ' && raw.trim() == "}" {
            break;
        }
        end += 1;
    }
    if end >= lines.len() {
        return Err(format!("mir: unterminated fn `{name}`"));
    }
    let body = &lines[start + 1..end];

    let mut locals: Vec<LocalDecl> = Vec::new();
    let mut ensure = |n: u32, locals: &mut Vec<LocalDecl>| {
        while locals.len() <= n as usize {
            locals.push(LocalDecl { ty: Box::new(Ty::Unit), is_arg: false, name: None });
        }
    };

    // Locals, in whatever scope they are declared.
    for line in body {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("debug ") {
            if let Some((nm, rhs)) = rest.split_once("=>") {
                if let Some(n) = parse_local_id(rhs.trim()) {
                    ensure(n, &mut locals);
                    if locals[n as usize].name.is_none() {
                        locals[n as usize].name = Some(nm.trim().to_string());
                    }
                }
            }
            continue;
        }
        if let Some((n, ty)) = parse_let(t) {
            ensure(n, &mut locals);
            locals[n as usize].ty = Box::new(ty);
        }
    }
    // Arguments override the declared types (the header is authoritative).
    // They live at their own local index, which MIR numbers from `_1`.
    let n_args = arg_tys.len() as u32;
    for (idx, ty) in arg_tys {
        ensure(idx, &mut locals);
        locals[idx as usize].ty = Box::new(ty);
        locals[idx as usize].is_arg = true;
    }
    ensure(n_args, &mut locals);

    // Blocks.
    let mut blocks: Vec<BasicBlock> = Vec::new();
    let mut cur: Option<u32> = None;
    for line in body {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(n) = parse_bb_header(t) {
            ensure_blocks(&mut blocks, n);
            blocks[n as usize] = BasicBlock { stmts: Vec::new(), term: Terminator::Unreachable };
            cur = Some(n);
            continue;
        }
        if t == "}" {
            cur = None;
            continue;
        }
        let Some(n) = cur else { continue };
        // A terminator line (`return;`, `goto -> bbX`, `switchInt(..)`, a call
        // or `assert(..)`) is not a statement; try it first.
        if let Some(term) = parse_call_terminator(t).or_else(|| parse_terminator(t)) {
            blocks[n as usize].term = term;
            continue;
        }
        match parse_line_stmt(t) {
            Ok(Some(s)) => blocks[n as usize].stmts.push(s),
            Ok(None) => {}
            Err(e) => return Err(format!("mir: {e} (in `{name}`): {t}")),
        }
    }

    let f = Function { name, locals, blocks, consts: HashMap::new() };
    Ok((f, end + 1))
}

fn ensure_blocks(blocks: &mut Vec<BasicBlock>, n: u32) {
    while blocks.len() <= n as usize {
        blocks.push(BasicBlock { stmts: Vec::new(), term: Terminator::Unreachable });
    }
}

/// `bb12: {` -> `Some(12)`.
fn parse_bb_header(t: &str) -> Option<u32> {
    let r = t.strip_prefix("bb")?;
    let colon = r.find(':')?;
    r[..colon].trim().parse().ok()
}

/// `let mut _2: Wide;` -> `(2, Wide)`. A `let` with a value is skipped.
fn parse_let(t: &str) -> Option<(u32, Ty)> {
    let t = t.strip_prefix("let ")?;
    let t = t.strip_prefix("mut ").unwrap_or(t);
    let t = t.strip_prefix('_')?;
    let colon = t.find(':')?;
    let n: u32 = t[..colon].trim().parse().ok()?;
    let rest = t[colon + 1..].trim().trim_end_matches(';').trim();
    if rest.contains('=') {
        return None;
    }
    Some((n, parse_ty(rest).ok()?))
}

/// `_12` -> `Some(12)`.
fn parse_local_id(s: &str) -> Option<u32> {
    let s = s.trim().trim_end_matches(';').trim();
    s.strip_prefix('_')?.parse().ok()
}

/// `bb12` -> `Some(12)`.
fn parse_bb_id(s: &str) -> Option<u32> {
    let s = s.trim().trim_end_matches(';').trim();
    s.strip_prefix("bb")?.parse().ok()
}

/// Parse one body line as a statement, if it is one. A call is a terminator,
/// not a statement, and is handled by [`parse_call_terminator`].
fn parse_line_stmt(t: &str) -> Result<Option<Stmt>, String> {
    if is_ignorable(t) || parse_call_terminator(t).is_some() {
        return Ok(None);
    }
    if let Some((lhs, rhs)) = split_assign(t) {
        let dest = parse_place(lhs)?;
        let rv = parse_rvalue(rhs)?;
        return Ok(Some(Stmt::Assign(dest, rv)));
    }
    Err("unrecognised statement".to_string())
}

/// `_4 = f(args) -> [return: bbX, unwind ...]` is a call terminator.
fn parse_call_terminator(t: &str) -> Option<Terminator> {
    let (lhs, rhs) = split_assign(t)?;
    let (head, targets) = split_arrow(rhs)?;
    let (func, args) = parse_call_head(head).ok()?;
    let dest = parse_place(lhs).ok()?;
    let target = parse_success_target(targets);
    let func = if let Some(name) = normalize_call_name(&func) {
        CallTarget::Direct(name)
    } else {
        CallTarget::Indirect
    };
    Some(Terminator::Call { dest, func, args, target })
}

/// The name a call should target, or `None` for a call that is not a plain
/// direct call (a function pointer, or an unresolved generic instantiation).
///
/// An inherent or trait method prints qualified (`Wide::total`,
/// `<core::ops::Range<i64> as Iterator>::next`). rmc's frontend drops the
/// qualifier, so the last path segment is the definition name; a leading
/// `::`-less segment after stripping an `<...>` qualifier is that name.
fn normalize_call_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('_') {
        return None;
    }
    if !name.contains("::") && !name.contains('<') {
        return Some(name.to_string());
    }
    // Drop a leading `<...>` qualifier, then take the final path segment.
    let mut rest = name;
    if let Some(stripped) = rest.strip_prefix('<') {
        let close = matching(rest, 0, '<', '>')?;
        rest = &rest[close + 1..];
    }
    let last = rest.rsplit("::").next()?.trim();
    if last.is_empty() || last.starts_with('_') || last.contains('<') {
        return None;
    }
    Some(last.to_string())
}

fn is_ignorable(t: &str) -> bool {
    let kw = t.split('(').next().unwrap_or(t).trim();
    matches!(
        kw,
        "StorageLive" | "StorageDead" | "FakeRead" | "AscribeUserType" | "Coverage"
            | "Nop" | "Retag" | "PlaceMention" | "Deinit" | "ConstEvalCounter"
            | "SetDiscriminant" | "discriminant" | "debug" | "storage" | "var_debug_info"
            | "SourceScope" | "SourceScopeData"
    ) || t.starts_with("//")
}

/// `LHS = RHS;` split at the first top-level ` = `.
fn split_assign(t: &str) -> Option<(&str, &str)> {
    let t = t.trim_end_matches(';').trim();
    let mut depth = 0i32;
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'=' if depth == 0 => {
                // Must be a lone `=` (an assignment), not `==`, `<=`, `>=`, `=>`.
                let prev = if i > 0 { bytes[i - 1] } else { b' ' };
                let next = bytes.get(i + 1).copied().unwrap_or(b' ');
                if prev == b' ' && next == b' ' {
                    return Some((t[..i].trim(), t[i + 1..].trim()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split `f(args) -> [targets]` into `("f(args)", "[targets]")`.
fn split_arrow(t: &str) -> Option<(&str, &str)> {
    let idx = t.find("->")?;
    Some((t[..idx].trim(), t[idx + 2..].trim()))
}

/// `f(move _1, const 2)` -> `("f", [Move(_1), Const(2)])`.
fn parse_call_head(s: &str) -> Result<(String, Vec<Operand>), String> {
    let open = s.find('(').ok_or_else(|| format!("mir: bad call `{s}`"))?;
    let func = s[..open].trim().to_string();
    let close = matching(s, open, '(', ')').ok_or_else(|| format!("mir: unbalanced call `{s}`"))?;
    let args = split_top_args(&s[open + 1..close]);
    let mut ops = Vec::new();
    for a in args {
        if a.trim().is_empty() {
            continue;
        }
        ops.push(parse_operand(a.trim())?);
    }
    Ok((func, ops))
}

/// The return edge of a call: `[return: bbN, unwind ...]`. Older MIR prints
/// `success:` for the same edge; `unwind` edges are ignored (this backend does
/// not unwind).
fn parse_success_target(targets: &str) -> Option<u32> {
    let inner = targets.trim().trim_start_matches('[').trim_end_matches(']');
    for part in inner.split(',') {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix("return:").or_else(|| p.strip_prefix("success:")) {
            return parse_bb_id(rest.trim());
        }
    }
    None
}

fn parse_terminator(t: &str) -> Option<Terminator> {
    let t = t.trim();
    if let Some(rest) = t.strip_prefix("goto -> ") {
        return Some(Terminator::Goto(parse_bb_id(rest.trim())?));
    }
    if t.starts_with("switchInt(") {
        return Some(parse_switch(t).ok()?);
    }
    if t == "return;" || t == "return" {
        return Some(Terminator::Return);
    }
    if t.starts_with("unreachable") {
        return Some(Terminator::Unreachable);
    }
    if t.starts_with("assert(") {
        // `assert(COND, "msg") -> [success: bbX, unwind ...]`. The condition is
        // an operand that may be negated (`!move _6`); a false condition traps.
        let open = t.find('(')?;
        let close = matching(t, open, '(', ')')?;
        let inner = &t[open + 1..close];
        let cond_part = split_top_args(inner).into_iter().next()?;
        let cond_part = cond_part.trim();
        let (negate, rest) = match cond_part.strip_prefix('!') {
            Some(r) => (true, r.trim()),
            None => (false, cond_part),
        };
        let cond = parse_operand(rest).ok()?;
        let (_, targets) = split_arrow(t)?;
        let target = parse_success_target(targets)?;
        return Some(Terminator::Assert { cond, negate, target });
    }
    if t.starts_with("resume") {
        return Some(Terminator::Abort);
    }
    if let Some(rest) = t.strip_prefix("drop(") {
        // `drop(_N) -> [return: bbX, ...]`
        if let Some((_, targets)) = split_arrow(t) {
            if let Some(bb) = parse_success_target(targets) {
                return Some(Terminator::Goto(bb));
            }
            let _ = rest;
            return Some(Terminator::Unreachable);
        }
        return Some(Terminator::Unreachable);
    }
    if t.starts_with("StorageLive") || t.starts_with("StorageDead") || t == "Nop;" {
        return None;
    }
    None
}

fn parse_switch(t: &str) -> Result<Terminator, String> {
    let open = t.find('(').ok_or("mir: bad switchInt")?;
    let close = matching(t, open, '(', ')').ok_or("mir: unbalanced switchInt")?;
    let discr = parse_operand(t[open + 1..close].trim())?;
    let after = t[close + 1..].trim();
    let after = after.strip_prefix("->").ok_or("mir: switchInt missing ->")?.trim();
    let inner = after.trim().trim_start_matches('[').trim_end_matches(';').trim_end_matches(']');
    let mut targets = Vec::new();
    let mut otherwise = u32::MAX;
    for part in inner.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if let Some(v) = p.strip_prefix("otherwise:") {
            otherwise = parse_bb_id(v.trim()).ok_or("mir: bad otherwise")?;
        } else if let Some((val, bb)) = p.split_once(':') {
            let v = parse_const(val.trim()).map(|c| const_as_i64(&c)).unwrap_or(0);
            let b = parse_bb_id(bb.trim()).ok_or("mir: bad switch target")?;
            targets.push((v, b));
        }
    }
    Ok(Terminator::SwitchInt { discr, targets, otherwise })
}

fn const_as_i64(c: &Const) -> i64 {
    match c {
        Const::Int(n) => *n,
        Const::Bool(b) => *b as i64,
        _ => 0,
    }
}

/// Parse a place: `_3`, `(*_1).0`, `(*_9)[0 of 1]`, `_2[3 of 4]`, `(*_1)[_2]`,
/// or the annotated form `((*_1).0: i64)`.
fn parse_place(s: &str) -> Result<Place, String> {
    let s = s.trim();
    // `(*_N)` — a dereference of the pointer in local N.
    if let Some(rest) = s.strip_prefix("(*") {
        let close = rest.find(')').ok_or_else(|| format!("mir: bad deref `{s}`"))?;
        let local =
            parse_local_id(&rest[..close]).ok_or_else(|| format!("mir: bad deref local `{s}`"))?;
        let mut proj = vec![Proj::Deref];
        proj.extend(parse_projections(&rest[close + 1..])?);
        return Ok(Place { local, proj });
    }
    // `(PLACE: TY)` — a type-annotated place. Strip the annotation and recurse.
    if s.starts_with('(') {
        let close = matching(s, 0, '(', ')').ok_or_else(|| format!("mir: unbalanced place `{s}`"))?;
        let inner = strip_top_type(&s[1..close]);
        let mut pl = parse_place(inner)?;
        pl.proj.extend(parse_projections(&s[close + 1..])?);
        return Ok(pl);
    }
    if s.starts_with('_') {
        let end = s[1..].find(|c: char| !c.is_ascii_digit()).map(|i| i + 1).unwrap_or(s.len());
        let local = parse_local_id(&s[..end]).ok_or_else(|| format!("mir: bad local `{s}`"))?;
        let proj = parse_projections(&s[end..])?;
        return Ok(Place { local, proj });
    }
    Err(format!("mir: not a place `{s}`"))
}

/// The `.field` and `[index]` chain after a place base.
fn parse_projections(mut rest: &str) -> Result<Vec<Proj>, String> {
    let mut proj = Vec::new();
    loop {
        let r = rest.trim_start();
        if let Some(r2) = r.strip_prefix('.') {
            let end = r2.find(|c: char| !c.is_ascii_digit()).unwrap_or(r2.len());
            let f: u32 = r2[..end].parse().map_err(|_| format!("mir: bad field `{rest}`"))?;
            proj.push(Proj::Field(f));
            rest = &r2[end..];
        } else if let Some(r2) = r.strip_prefix('[') {
            let close = r2.find(']').ok_or_else(|| format!("mir: bad index `{rest}`"))?;
            let inner = r2[..close].trim();
            // `_2[3 of 4]` names a constant; `(*_1)[_2]` a local.
            let idx = inner.split(" of ").next().unwrap_or(inner).trim();
            if idx.starts_with('_') {
                proj.push(Proj::IndexLocal(
                    parse_local_id(idx).ok_or_else(|| format!("mir: bad index local `{rest}`"))?,
                ));
            } else if let Some(n) = parse_int_literal(idx) {
                proj.push(Proj::IndexConst(n as u64));
            } else {
                return Err(format!("mir: unsupported index `{rest}`"));
            }
            rest = &r2[close + 1..];
        } else {
            break;
        }
    }
    Ok(proj)
}

/// `PLACE: TY` -> `PLACE`. The annotation sits at paren/bracket depth 0.
fn strip_top_type(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b':' if depth == 0 => {
                if bytes.get(i + 1) == Some(&b':') {
                    i += 1;
                } else {
                    return s[..i].trim();
                }
            }
            _ => {}
        }
        i += 1;
    }
    s.trim()
}

fn parse_rvalue(s: &str) -> Result<Rvalue, String> {
    let s = s.trim();
    // `no_retag` marks a value that the borrow checker does not retag; it does
    // not change the value itself.
    if let Some(r) = s.strip_prefix("no_retag ") {
        return parse_rvalue(r.trim());
    }
    // `&raw const PLACE` / `&raw mut PLACE`
    if let Some(r) = s.strip_prefix("&raw const ") {
        return Ok(Rvalue::RawRef { mutbl: false, place: parse_place(r.trim())? });
    }
    if let Some(r) = s.strip_prefix("&raw mut ") {
        return Ok(Rvalue::RawRef { mutbl: true, place: parse_place(r.trim())? });
    }
    if let Some(r) = s.strip_prefix("&mut ") {
        return Ok(Rvalue::Ref { mutbl: true, place: parse_place(r.trim())? });
    }
    if let Some(r) = s.strip_prefix('&') {
        // `&PLACE`; guard against `&&` which does not appear.
        if !r.starts_with('&') {
            return Ok(Rvalue::Ref { mutbl: false, place: parse_place(r.trim())? });
        }
    }
    // A cast: `OPERAND as TY (KIND)`.
    if let Some((op_part, ty_kind)) = split_top_as(s) {
        let (ty, kind) = parse_cast_target(ty_kind)?;
        let op = parse_operand(op_part.trim())?;
        let from = operand_ty(&op);
        return Ok(Rvalue::Cast { from: Box::new(from), to: Box::new(ty), op, kind });
    }
    // A bare constant operand, e.g. `const {alloc2: &&[u8]}` or `const 0_i64`.
    if let Some(r) = s.strip_prefix("const ") {
        let c = parse_const(r.trim()).ok_or_else(|| format!("mir: bad const `{s}`"))?;
        return Ok(Rvalue::Use(Operand::Const(c)));
    }
    if s == "[]" {
        return Ok(Rvalue::Array(Vec::new()));
    }
    if s.starts_with('[') {
        // `[v; N]` repeat or `[a, b, c]` list.
        let close = matching(s, 0, '[', ']').ok_or_else(|| format!("mir: unbalanced array `{s}`"))?;
        let inner = &s[1..close];
        if let Some((v, n)) = split_top_semi(inner) {
            let count = parse_int_literal(n.trim()).ok_or_else(|| format!("mir: bad repeat count `{s}`"))?;
            return Ok(Rvalue::Repeat { op: parse_operand(v.trim())?, count: count as u64 });
        }
        let parts = split_top_args(inner);
        let mut ops = Vec::new();
        for p in parts {
            if p.trim().is_empty() {
                continue;
            }
            ops.push(parse_operand(p.trim())?);
        }
        return Ok(Rvalue::Array(ops));
    }
    // A struct/tuple literal: `Name { f: v, .. }` or `Name(v, ..)`.
    if let Some(brace) = s.find('{') {
        let name = s[..brace].trim();
        if !name.is_empty() && !name.contains("const ") && !name.contains('(') {
            let close = matching(s, brace, '{', '}').ok_or_else(|| format!("mir: unbalanced aggregate `{s}`"))?;
            let inner = &s[brace + 1..close];
            let mut ops = Vec::new();
            for part in split_top_args(inner) {
                let p = part.trim();
                if p.is_empty() {
                    continue;
                }
                // `field: OPERAND`
                if let Some((_, v)) = p.split_once(':') {
                    ops.push(parse_operand(v.trim())?);
                } else {
                    ops.push(parse_operand(p)?);
                }
            }
            return Ok(Rvalue::Aggregate(ops));
        }
    }
    // An enum variant constructor `Enum::Variant { .. }` was handled above; a
    // unit variant `Enum::Variant` is a bare name.
    // `Len(PLACE)`
    if let Some(r) = s.strip_prefix("Len(") {
        let inner = r.trim_end_matches(')');
        return Ok(Rvalue::Len(parse_place(inner.trim())?));
    }
    // `discriminant(PLACE)`
    if let Some(r) = s.strip_prefix("discriminant(") {
        let inner = r.trim_end_matches(')');
        return Ok(Rvalue::Discriminant(parse_place(inner.trim())?));
    }
    // `AddWithOverflow(a, b)` / `SubWithOverflow` / `MulWithOverflow`.
    for (name, op) in [("AddWithOverflow", BinOp::Add), ("SubWithOverflow", BinOp::Sub), ("MulWithOverflow", BinOp::Mul)] {
        if let Some(r) = s.strip_prefix(name) {
            let r = r.trim();
            if let Some(open) = r.find('(') {
                let close = matching(r, open, '(', ')').ok_or_else(|| format!("mir: unbalanced op `{s}`"))?;
                let args = split_top_args(&r[open + 1..close]);
                if args.len() == 2 {
                    return Ok(Rvalue::WithOverflow {
                        op,
                        a: parse_operand(args[0].trim())?,
                        b: parse_operand(args[1].trim())?,
                    });
                }
            }
        }
    }
    // `f(args)` without a `->` is a function-pointer value; unsupported.
    // Binary and unary ops: `Op(a, b)`.
    if let Some(open) = s.find('(') {
        let opname = s[..open].trim();
        if let Some(op) = bin_op(opname) {
            let close = matching(s, open, '(', ')').ok_or_else(|| format!("mir: unbalanced op `{s}`"))?;
            let args = split_top_args(&s[open + 1..close]);
            if args.len() == 2 {
                return Ok(Rvalue::Binary(op, parse_operand(args[0].trim())?, parse_operand(args[1].trim())?));
            }
        }
        if let Some(op) = un_op(opname) {
            let close = matching(s, open, '(', ')').ok_or_else(|| format!("mir: unbalanced unop `{s}`"))?;
            let arg = s[open + 1..close].trim();
            return Ok(Rvalue::Unary(op, parse_operand(arg)?));
        }
    }
    // Fall back to an operand (a `copy`/`move`/`const`).
    if s.starts_with("copy ") || s.starts_with("move ") || s.starts_with("const ") {
        return Ok(Rvalue::Use(parse_operand(s)?));
    }
    // A bare local or place used as a value.
    if s.starts_with('_') || s.starts_with("(*") {
        return Ok(Rvalue::Use(Operand::Copy(parse_place(s)?)));
    }
    Ok(Rvalue::Unsupported(s.to_string()))
}

fn operand_ty(op: &Operand) -> Ty {
    match op {
        Operand::Const(Const::Bool(_)) => Ty::Bool,
        _ => Ty::I64,
    }
}

fn bin_op(name: &str) -> Option<BinOp> {
    Some(match name {
        "Add" | "UncheckedAdd" => BinOp::Add,
        "Sub" | "UncheckedSub" => BinOp::Sub,
        "Mul" | "UncheckedMul" => BinOp::Mul,
        "Div" => BinOp::Div,
        "Rem" => BinOp::Rem,
        "BitXor" => BinOp::BitXor,
        "BitAnd" => BinOp::BitAnd,
        "BitOr" => BinOp::BitOr,
        "Shl" | "UncheckedShl" => BinOp::Shl,
        "Shr" | "UncheckedShr" => BinOp::Shr,
        "Eq" => BinOp::Eq,
        "Ne" => BinOp::Ne,
        "Lt" => BinOp::Lt,
        "Le" => BinOp::Le,
        "Gt" => BinOp::Gt,
        "Ge" => BinOp::Ge,
        "Cmp" => BinOp::Cmp,
        "Offset" => BinOp::Offset,
        "PtrOffset" => BinOp::PtrOffset,
        _ => return None,
    })
}

fn un_op(name: &str) -> Option<UnOp> {
    Some(match name {
        "Not" => UnOp::Not,
        "Neg" => UnOp::Neg,
        "PtrMetadata" => UnOp::PtrMetadata,
        _ => return None,
    })
}

fn parse_cast_target(s: &str) -> Result<(Ty, CastKind), String> {
    let s = s.trim();
    // `TY (KIND)` — the kind is the last parenthesised group.
    let (ty_str, kind) = if let Some(open) = s.rfind('(') {
        let ty = s[..open].trim();
        let kind = s[open + 1..].trim().trim_end_matches(')').trim();
        (ty, cast_kind(kind))
    } else {
        (s, CastKind::Other)
    };
    Ok((parse_ty(ty_str)?, kind))
}

fn cast_kind(k: &str) -> CastKind {
    match k {
        "IntToInt" => CastKind::IntToInt,
        "PtrToPtr" | "PointerWithExposedProvenance" | "PtrToPtrCast" => CastKind::PtrToPtr,
        "IntToPtr" => CastKind::IntToPtr,
        "PtrToInt" => CastKind::PtrToInt,
        _ => CastKind::Other,
    }
}

fn parse_operand(s: &str) -> Result<Operand, String> {
    let s = s.trim();
    if let Some(r) = s.strip_prefix("copy ") {
        return Ok(Operand::Copy(parse_place(r.trim())?));
    }
    if let Some(r) = s.strip_prefix("move ") {
        return Ok(Operand::Move(parse_place(r.trim())?));
    }
    if let Some(r) = s.strip_prefix("const ") {
        return Ok(Operand::Const(
            parse_const(r.trim()).ok_or_else(|| format!("mir: bad const `{s}`"))?,
        ));
    }
    // A bare place.
    if s.starts_with('_') || s.starts_with("(*") {
        return Ok(Operand::Copy(parse_place(s)?));
    }
    Err(format!("mir: not an operand `{s}`"))
}

fn parse_const(s: &str) -> Option<Const> {
    let s = s.trim();
    if s == "()" {
        return Some(Const::Unit);
    }
    if s == "true" {
        return Some(Const::Bool(true));
    }
    if s == "false" {
        return Some(Const::Bool(false));
    }
    if s.starts_with('"') {
        let inner = s.trim_start_matches('"').trim_end_matches('"');
        return Some(Const::Str(unescape(inner)));
    }
    if let Some(n) = parse_named_const(s) {
        return Some(Const::Int(n));
    }
    if s.starts_with('{') {
        // `{allocN: TY}` — a static allocation, possibly with an interior
        // offset printed as `{allocN: TY[place]}`.
        if let Some(rest) = s.strip_prefix("{alloc") {
            if let Some(colon) = rest.find(':') {
                if let Ok(id) = rest[..colon].trim().parse::<u32>() {
                    let after = &rest[colon + 1..];
                    let place = after
                        .find('[')
                        .and_then(|open| matching(after, open, '[', ']').map(|close| &after[open + 1..close]))
                        .and_then(|p| parse_place(p).ok())
                        .map(Box::new);
                    return Some(Const::Alloc { id, place });
                }
            }
        }
        return Some(Const::Named(s.to_string()));
    }
    if let Some(n) = parse_int_literal(s) {
        return Some(Const::Int(n));
    }
    None
}

/// `i64::MIN`, `u64::MAX`, `usize::MAX`, `i32::MIN`, ... — the associated
/// integer constants MIR prints by name.
fn parse_named_const(s: &str) -> Option<i64> {
    let (ty, name) = s.split_once("::")?;
    let bits: u32 = match ty {
        "i8" | "u8" => 8,
        "i16" | "u16" => 16,
        "i32" | "u32" => 32,
        "i64" | "u64" => 64,
        "isize" | "usize" => 64,
        _ => return None,
    };
    let signed = ty.starts_with('i');
    match name {
        "MIN" if signed => Some(-(1i64 << (bits - 1))),
        "MAX" if signed => Some((1i64 << (bits - 1)) - 1),
        "MAX" => Some(-1), // unsigned max fits the 64-bit word as all ones
        _ => None,
    }
}

/// `0_i64`, `100_usize`, `-1_i64`, `0x10_u8`, `0x8000000000000000_u64`.
fn parse_int_literal(s: &str) -> Option<i64> {
    let s = s.trim();
    let (neg, body) = if let Some(r) = s.strip_prefix('-') { (true, r) } else { (false, s) };
    // A `_TYPE` suffix is present when the part before the last `_` parses as a
    // number; try the stripped form first and fall back to the whole string.
    let cand = match body.rfind('_') {
        Some(i) => &body[..i],
        None => body,
    };
    let v = parse_uint(cand).or_else(|| parse_uint(body))?;
    Some(if neg { (v as i64).wrapping_neg() } else { v as i64 })
}

fn parse_uint(num: &str) -> Option<u64> {
    let num = num.trim();
    if let Some(h) = num.strip_prefix("0x") {
        u64::from_str_radix(h, 16).ok()
    } else if let Some(h) = num.strip_prefix("0b") {
        u64::from_str_radix(h, 2).ok()
    } else if let Some(h) = num.strip_prefix("0o") {
        u64::from_str_radix(h, 8).ok()
    } else {
        num.parse::<u64>().ok().or_else(|| num.parse::<i64>().ok().map(|v| v as u64))
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Function arguments, as `(local index, type)` pairs. The index matters: MIR
/// numbers arguments from `_1` (`_0` is the return place), and the locals are
/// not necessarily declared in index order.
fn parse_arg_tys(s: &str) -> Result<Vec<(u32, Ty)>, String> {
    let mut out = Vec::new();
    for part in split_top_args(s) {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        // `_1: i64`
        let colon = p.find(':').ok_or_else(|| format!("mir: bad arg `{p}`"))?;
        let idx = parse_local_id(&p[..colon]).ok_or_else(|| format!("mir: bad arg `{p}`"))?;
        out.push((idx, parse_ty(p[colon + 1..].trim())?));
    }
    Ok(out)
}

pub fn parse_ty(s: &str) -> Result<Ty, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(Ty::Unit);
    }
    if let Some(r) = s.strip_prefix("&mut ") {
        return Ok(Ty::Ref(Box::new(parse_ty(r)?)));
    }
    if let Some(r) = s.strip_prefix("&") {
        // `&[T]` is a fat pointer; treat as a plain pointer to the element.
        return Ok(Ty::Ref(Box::new(parse_ty(r)?)));
    }
    if let Some(r) = s.strip_prefix("*const ") {
        return Ok(Ty::Ptr(Box::new(parse_ty(r)?)));
    }
    if let Some(r) = s.strip_prefix("*mut ") {
        return Ok(Ty::Ptr(Box::new(parse_ty(r)?)));
    }
    if s.starts_with('[') {
        let close = matching(s, 0, '[', ']').ok_or_else(|| format!("mir: bad array ty `{s}`"))?;
        let inner = &s[1..close];
        if let Some((elem, n)) = split_top_semi(inner) {
            let len = parse_int_literal(n.trim()).unwrap_or(0) as u64;
            return Ok(Ty::Array(Box::new(parse_ty(elem.trim())?), len));
        }
        // A slice `[T]`; a pointer to it is just a pointer.
        return Ok(Ty::Array(Box::new(parse_ty(inner.trim())?), 0));
    }
    if s.starts_with('(') {
        let close = matching(s, 0, '(', ')').ok_or_else(|| format!("mir: bad tuple ty `{s}`"))?;
        let inner = &s[1..close];
        let parts = split_top_args(inner);
        if parts.iter().all(|p| p.trim().is_empty()) {
            return Ok(Ty::Unit);
        }
        let mut tys = Vec::new();
        for p in parts {
            tys.push(Box::new(parse_ty(p.trim())?));
        }
        return Ok(Ty::Tuple(tys));
    }
    match s {
        "i64" => Ok(Ty::I64),
        "bool" => Ok(Ty::Bool),
        "usize" | "isize" | "u64" | "u32" | "i32" | "u16" | "i16" | "u8" | "i8"
        | "u128" | "i128" | "char" => Ok(Ty::Int),
        "()" => Ok(Ty::Unit),
        "!" => Ok(Ty::Unit),
        _ => {
            if s.contains("fn(") || s.contains("dyn ") || s.contains("::") {
                Ok(Ty::Other)
            } else {
                Ok(Ty::Struct(s.to_string()))
            }
        }
    }
}

// --- small string utilities ------------------------------------------------

/// Index of the matching close bracket for the opener at `open`.
fn matching(s: &str, open: usize, oc: char, cc: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < open) {
        if c == oc {
            depth += 1;
        } else if c == cc {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Split a comma-separated list at the top nesting level.
fn split_top_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | '[' | '{' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Split at the top-level `;` (for `[value; count]` and `[T; N]`).
fn split_top_semi(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ';' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Inlining
// ---------------------------------------------------------------------------

/// How many inlining rounds to run. Each round can expose fresh call sites (a
/// call in a body that was itself inlined), so a couple of rounds are enough to
/// flatten the small accessor chains the benchmarks use.
const INLINE_ROUNDS: usize = 3;
/// LICM runs to a fixpoint, but the chain of loads an inlined accessor leaves
/// can be deep (one round per level), so the cap is generous; the loop exits as
/// soon as a round hoists nothing.
const LICM_ROUNDS: usize = 64;
/// Never inline a body larger than this many blocks; it would grow the caller
/// without bound and slow the compile down.
const INLINE_MAX_BLOCKS: usize = 48;
/// Stop inlining into a function once it grows past this many blocks. Guards
/// against a deep call chain multiplying the body out of control.
const INLINE_FUNC_MAX: usize = 400;

/// Inline direct calls, repeatedly, until no more calls can be inlined. A call
/// is inlined only when the callee is small, is not (transitively) able to call
/// back into the caller, and every argument is a plain operand.
///
/// Inlining is what lets the AST backend's `licm` see through the `&self`
/// accessors in `refs`; rustc leaves those calls in place at `-O`, so the MIR
/// backend has to do it here.
pub fn inline_calls(prog: &mut Program) {
    for _ in 0..INLINE_ROUNDS {
        let index: HashMap<String, Function> =
            prog.funcs.iter().map(|f| (f.name.clone(), f.clone())).collect();
        let reach = reachability(&index);
        let mut changed = false;
        for f in &mut prog.funcs {
            let (blocks, locals, did) = inline_func(f, &index, &reach);
            if did {
                f.blocks = blocks;
                f.locals = locals;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// For each function, the set of functions it can reach through direct calls
/// (not including itself unless it recurses).
fn reachability(index: &HashMap<String, Function>) -> HashMap<String, HashSet<String>> {
    let mut reach = HashMap::new();
    for name in index.keys() {
        let mut seen: HashSet<String> = HashSet::new();
        let mut stack = vec![name.clone()];
        while let Some(n) = stack.pop() {
            let Some(f) = index.get(&n) else { continue };
            for b in &f.blocks {
                if let Terminator::Call { func: CallTarget::Direct(c), .. } = &b.term {
                    if seen.insert(c.clone()) {
                        stack.push(c.clone());
                    }
                }
            }
        }
        reach.insert(name.clone(), seen);
    }
    reach
}

/// Inline every eligible call in `f`, returning the rewritten body. `did` is
/// false when nothing changed, so the caller can stop iterating.
fn inline_func(
    f: &Function,
    index: &HashMap<String, Function>,
    reach: &HashMap<String, HashSet<String>>,
) -> (Vec<BasicBlock>, Vec<LocalDecl>, bool) {
    let mut blocks: Vec<BasicBlock> = f.blocks.clone();
    let mut locals: Vec<LocalDecl> = f.locals.clone();
    let mut did = false;
    let mut bb = 0;
    // New blocks are appended as calls are inlined; scanning them too lets a
    // call inside an inlined body be inlined in the same round.
    while bb < blocks.len() {
        if blocks.len() > INLINE_FUNC_MAX {
            break;
        }
        let Terminator::Call { dest, func, args, target: Some(target) } = blocks[bb].term.clone()
        else {
            bb += 1;
            continue;
        };
        let CallTarget::Direct(name) = &func else {
            bb += 1;
            continue;
        };
        if !can_inline(f, name, &dest, &args, &locals, index, reach) {
            bb += 1;
            continue;
        }
        let callee = &index[name];
        do_inline(&mut blocks, &mut locals, bb, callee, &dest, &args, target);
        did = true;
        bb += 1;
    }
    (blocks, locals, did)
}

fn can_inline(
    caller: &Function,
    name: &str,
    dest: &Place,
    args: &[Operand],
    locals: &[LocalDecl],
    index: &HashMap<String, Function>,
    reach: &HashMap<String, HashSet<String>>,
) -> bool {
    if name == caller.name {
        return false;
    }
    let Some(callee) = index.get(name) else { return false };
    // The callee must not be able to call back into the caller, or inlining
    // would turn the caller recursive (and unroll forever). A self-recursive
    // callee is refused for the same reason: its inlined copy would still hold
    // the recursive call, which the next round would inline again.
    if let Some(r) = reach.get(name) {
        if r.contains(&caller.name) || r.contains(name) {
            return false;
        }
    }
    if callee.blocks.len() > INLINE_MAX_BLOCKS {
        return false;
    }
    // A scalar destination keeps the return-value rewrite simple.
    if !dest.proj.is_empty() || dest.local as usize >= locals.len() {
        return false;
    }
    if callee.local_ty(0).is_aggregate() {
        return false;
    }
    let params = callee_arg_locals(callee);
    if params.len() != args.len() {
        return false;
    }
    // A body the backend cannot model would just move the failure inward.
    callee.blocks.iter().all(|b| !block_has_unsupported(b))
}

fn do_inline(
    blocks: &mut Vec<BasicBlock>,
    locals: &mut Vec<LocalDecl>,
    bb: usize,
    callee: &Function,
    dest: &Place,
    args: &[Operand],
    target: u32,
) {
    let base = locals.len() as u32;
    let block_base = blocks.len() as u32;
    let join = block_base + callee.blocks.len() as u32;
    let ctx = Remap { base, block_base, join };

    // The callee's locals are copied in with fresh indices; none of them is an
    // argument of the caller.
    for l in &callee.locals {
        locals.push(LocalDecl { ty: l.ty.clone(), is_arg: false, name: None });
    }

    // Parameter assignments, prepended to the callee entry block so the body
    // reads `_1`, `_2`, ... as before. The arguments are the *caller's*
    // operands, so they are copied verbatim, not remapped.
    let mut entry_stmts: Vec<Stmt> = Vec::new();
    for (p, a) in callee_arg_locals(callee).iter().zip(args.iter()) {
        entry_stmts.push(Stmt::Assign(
            Place { local: base + *p, proj: Vec::new() },
            Rvalue::Use(a.clone()),
        ));
    }

    // Clone and remap the callee body. `_0` becomes the caller's destination,
    // and every `return` jumps to the join block.
    let dest_local = dest.local;
    let mut new_blocks: Vec<BasicBlock> = Vec::with_capacity(callee.blocks.len() + 1);
    for (i, b) in callee.blocks.iter().enumerate() {
        let mut stmts: Vec<Stmt> = b.stmts.iter().map(|s| ctx.stmt(s, dest_local)).collect();
        if i == 0 {
            let mut v = entry_stmts.clone();
            v.append(&mut stmts);
            stmts = v;
        }
        new_blocks.push(BasicBlock { stmts, term: ctx.term(&b.term, dest_local) });
    }
    // The join block returns to the original continuation.
    new_blocks.push(BasicBlock { stmts: Vec::new(), term: Terminator::Goto(target) });

    // The call site becomes a jump into the inlined entry block.
    blocks[bb].term = Terminator::Goto(block_base);
    blocks.extend(new_blocks);
}

/// The locals a function's signature binds as parameters, in order.
fn callee_arg_locals(f: &Function) -> Vec<u32> {
    let mut params: Vec<u32> = f
        .locals
        .iter()
        .enumerate()
        .filter(|(_, l)| l.is_arg)
        .map(|(i, _)| i as u32)
        .collect();
    params.sort_unstable();
    params
}

/// Remapping of a callee's locals and blocks into the caller's namespace.
struct Remap {
    base: u32,
    block_base: u32,
    join: u32,
}

impl Remap {
    fn block(&self, b: u32) -> u32 {
        self.block_base + b
    }

    /// A callee local maps to a fresh caller local; the return slot (`_0`) is
    /// left alone here and redirected by `place_ret`.
    fn place(&self, p: &Place) -> Place {
        Place {
            local: if p.local == 0 { 0 } else { self.base + p.local },
            proj: p
                .proj
                .iter()
                .map(|pr| match pr {
                    Proj::IndexLocal(l) => Proj::IndexLocal(self.base + l),
                    other => other.clone(),
                })
                .collect(),
        }
    }

    /// Like `place`, but a reference to the callee's `_0` is redirected to the
    /// caller's call destination.
    fn place_ret(&self, p: &Place, dest_local: u32) -> Place {
        let mut q = self.place(p);
        if p.local == 0 {
            q.local = dest_local;
        }
        q
    }

    fn op(&self, o: &Operand) -> Operand {
        match o {
            Operand::Copy(p) => Operand::Copy(self.place(p)),
            Operand::Move(p) => Operand::Move(self.place(p)),
            Operand::Const(c) => Operand::Const(self.const_(c)),
        }
    }

    fn op_ret(&self, o: &Operand, dest_local: u32) -> Operand {
        match o {
            Operand::Copy(p) => Operand::Copy(self.place_ret(p, dest_local)),
            Operand::Move(p) => Operand::Move(self.place_ret(p, dest_local)),
            Operand::Const(c) => Operand::Const(self.const_(c)),
        }
    }

    fn const_(&self, c: &Const) -> Const {
        match c {
            Const::Alloc { id, place } => Const::Alloc {
                id: *id,
                place: place.as_ref().map(|p| Box::new(self.place(p))),
            },
            other => other.clone(),
        }
    }

    fn stmt(&self, s: &Stmt, dest_local: u32) -> Stmt {
        match s {
            Stmt::Nop => Stmt::Nop,
            Stmt::Assign(p, rv) => {
                Stmt::Assign(self.place_ret(p, dest_local), self.rv(rv, dest_local))
            }
        }
    }

    fn rv(&self, rv: &Rvalue, dest_local: u32) -> Rvalue {
        match rv {
            Rvalue::Use(o) => Rvalue::Use(self.op_ret(o, dest_local)),
            Rvalue::Binary(op, a, b) => Rvalue::Binary(*op, self.op(a), self.op(b)),
            Rvalue::Unary(op, a) => Rvalue::Unary(*op, self.op(a)),
            Rvalue::Cast { from, to, op, kind } => Rvalue::Cast {
                from: from.clone(),
                to: to.clone(),
                op: self.op(op),
                kind: *kind,
            },
            Rvalue::Ref { mutbl, place } => {
                Rvalue::Ref { mutbl: *mutbl, place: self.place(place) }
            }
            Rvalue::RawRef { mutbl, place } => {
                Rvalue::RawRef { mutbl: *mutbl, place: self.place(place) }
            }
            Rvalue::Repeat { op, count } => Rvalue::Repeat { op: self.op(op), count: *count },
            Rvalue::Array(ops) => Rvalue::Array(ops.iter().map(|o| self.op(o)).collect()),
            Rvalue::Aggregate(ops) => Rvalue::Aggregate(ops.iter().map(|o| self.op(o)).collect()),
            Rvalue::Len(p) => Rvalue::Len(self.place(p)),
            Rvalue::WithOverflow { op, a, b } => {
                Rvalue::WithOverflow { op: *op, a: self.op(a), b: self.op(b) }
            }
            Rvalue::Discriminant(p) => Rvalue::Discriminant(self.place(p)),
            Rvalue::Unsupported(s) => Rvalue::Unsupported(s.clone()),
        }
    }

    fn term(&self, t: &Terminator, dest_local: u32) -> Terminator {
        match t {
            Terminator::Goto(b) => Terminator::Goto(self.block(*b)),
            Terminator::SwitchInt { discr, targets, otherwise } => Terminator::SwitchInt {
                discr: self.op(discr),
                targets: targets.iter().map(|(v, b)| (*v, self.block(*b))).collect(),
                otherwise: self.block(*otherwise),
            },
            Terminator::Return => Terminator::Goto(self.join),
            Terminator::Call { dest, func, args, target } => Terminator::Call {
                dest: self.place_ret(dest, dest_local),
                func: func.clone(),
                args: args.iter().map(|a| self.op(a)).collect(),
                target: target.map(|b| self.block(b)),
            },
            Terminator::Unreachable => Terminator::Unreachable,
            Terminator::Abort => Terminator::Abort,
            Terminator::Assert { cond, negate, target } => Terminator::Assert {
                cond: self.op(cond),
                negate: *negate,
                target: self.block(*target),
            },
        }
    }
}

/// True when a block contains an rvalue the backend cannot lower, so inlining it
/// would not help.
fn block_has_unsupported(b: &BasicBlock) -> bool {
    b.stmts.iter().any(|s| matches!(s, Stmt::Assign(_, Rvalue::Unsupported(_))))
}

fn split_top_as(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'a' if depth == 0 => {
                if s[i..].starts_with("as ") && i > 0 && bytes[i - 1] == b' ' {
                    // Do not treat `as` inside `*const`/`*mut` as a cast.
                    return Some((&s[..i - 1], &s[i + 3..]));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Loop-invariant code motion
// ---------------------------------------------------------------------------

/// The functions a MIR function may call without observable side effects, so a
/// call to one can be hoisted out of a loop. This is the MIR analogue of the
/// AST backend's `licm` pass, which runs after inlining; rustc leaves the
/// accessor calls in place, so they are hoisted here instead.
///
/// A call is only *hoisted* when every operand it reads is invariant and the
/// callee does not write the caller's frame, which the purity list plus the
/// no-memory-write rule below guarantee. Calls that allocate (`alloc`) or write
/// (`print_i64`, `putchar`) are never pure.
fn pure_call(name: &str) -> bool {
    !matches!(name, "alloc" | "print_i64" | "print_str" | "putchar" | "free")
}

/// Halve the call tree of a single self-recursive function.
///
/// `f(x) = t(x) op f(h(x))` recurses twice per level, so `fib(32)` makes
/// `2^32` calls. `op` is associative, so the `f(h(x))` spine can be unrolled
/// into an accumulator loop:
///
/// ```text
/// acc = identity                  // 0 for `+`, 1 for `*`
/// while !cond(x) { acc = acc op t(x); x = h(x); }
/// return acc op guard(x)
/// ```
///
/// One call per level remains — the one inside `t` — so the tree is halved.
/// This is the same rewrite `tco.rs` does on the AST; rustc emits the shape as
/// a chain of blocks (`t` may be a self-call, then `h` and a second self-call,
/// then the `op`), which is what the recogniser below walks.
///
/// The rewrite is only exact for wrapping arithmetic, so it is skipped under
/// `--overflow-checks`: the loop adds terms in a different order, and a checked
/// trap would fire on a different intermediate.
pub fn tco(prog: &mut Program, overflow_checks: bool) {
    if overflow_checks {
        return;
    }
    for f in &mut prog.funcs {
        f.tco_self();
    }
}

impl Function {
    fn tco_self(&mut self) -> bool {
        if self.blocks.is_empty() {
            return false;
        }
        // A free function of one scalar argument returning a scalar.
        let args: Vec<u32> = (0..self.locals.len() as u32)
            .filter(|l| self.locals[*l as usize].is_arg)
            .collect();
        if args.len() != 1 {
            return false;
        }
        let param = args[0];
        if self.locals[param as usize].ty.is_aggregate()
            || self.locals.first().map(|l| l.ty.is_aggregate()).unwrap_or(true)
        {
            return false;
        }
        // The entry block must be the only entry (no back edge into `bb0`).
        if self.blocks.iter().any(|b| successors(&b.term).contains(&0)) {
            return false;
        }

        // Entry: `discr = cmp(a, b); switchInt(discr) -> [0: X, otherwise: Y]`.
        let Terminator::SwitchInt { discr, targets, otherwise } = self.blocks[0].term.clone() else {
            return false;
        };
        if targets.len() != 1 || targets[0].0 != 0 {
            return false;
        }
        let discr_local = match &discr {
            Operand::Copy(p) | Operand::Move(p) if p.proj.is_empty() => p.local,
            _ => return false,
        };
        let Some((cmp_op, cmp_a, cmp_b)) = self.blocks[0].stmts.iter().find_map(|s| match s {
            Stmt::Assign(p, Rvalue::Binary(op, a, b))
                if p.proj.is_empty()
                    && p.local == discr_local
                    && matches!(
                        op,
                        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne
                    ) =>
            {
                Some((*op, a.clone(), b.clone()))
            }
            _ => None,
        }) else {
            return false;
        };

        let Some(ret_block) = self.blocks.iter().position(|b| matches!(b.term, Terminator::Return))
        else {
            return false;
        };
        let ret_block = ret_block as u32;

        // One switch arm is the base case: `_0 = guard; goto ret_block`.
        let arm_guard = |this: &Self, b: u32| -> Option<Operand> {
            let blk = &this.blocks[b as usize];
            if !matches!(blk.term, Terminator::Goto(t) if t == ret_block) {
                return None;
            }
            let mut guard = None;
            for s in &blk.stmts {
                if let Stmt::Assign(p, Rvalue::Use(op)) = s {
                    if p.proj.is_empty() && p.local == 0 {
                        if guard.is_some() {
                            return None;
                        }
                        guard = Some(op.clone());
                    }
                }
            }
            guard
        };
        let x = targets[0].1;
        let y = otherwise;
        let (base_block, chain_start, guard) = if let Some(g) = arm_guard(self, x) {
            (x, y, g)
        } else if let Some(g) = arm_guard(self, y) {
            (y, x, g)
        } else {
            return false;
        };

        // Walk the recursive spine from the entry arm to the `op` combine.
        let mut chain: Vec<u32> = Vec::new();
        let mut cur = chain_start;
        let combine;
        loop {
            if chain.len() > 64 {
                return false;
            }
            chain.push(cur);
            match self.blocks[cur as usize].term.clone() {
                Terminator::Goto(t) if t == ret_block => {
                    combine = cur;
                    break;
                }
                Terminator::Goto(t) => cur = t,
                Terminator::Call { target: Some(t), func: CallTarget::Direct(n), .. }
                    if n == self.name =>
                {
                    cur = t;
                }
                _ => return false,
            }
        }

        // The combine is `_0 = op(t, f(h))`; `op` must be associative.
        let Some((op, a, b)) = self.blocks[combine as usize].stmts.iter().find_map(|s| match s {
            Stmt::Assign(p, Rvalue::Binary(op @ (BinOp::Add | BinOp::Mul), a, b))
                if p.proj.is_empty() && p.local == 0 =>
            {
                Some((*op, a.clone(), b.clone()))
            }
            _ => None,
        }) else {
            return false;
        };

        // The last self-call on the spine is `f(h)`; its result is one operand
        // of the combine and its argument is `h`.
        let mut last_call = None;
        for &b in chain.iter().take(chain.len() - 1) {
            if let Terminator::Call { dest, args, func: CallTarget::Direct(n), .. } =
                &self.blocks[b as usize].term
            {
                if *n == self.name && args.len() == 1 {
                    last_call = Some((b, dest.local, args[0].clone()));
                }
            }
        }
        let Some((call_block, call_dest, h)) = last_call else { return false };
        // The combine must read the call's result exactly once, and its other
        // operand is the value of `t(x)`.
        let uses_dest = |o: &Operand| matches!(o, Operand::Copy(p) | Operand::Move(p) if p.proj.is_empty() && p.local == call_dest);
        let t_result = if uses_dest(&a) && !uses_dest(&b) {
            b
        } else if uses_dest(&b) && !uses_dest(&a) {
            a
        } else {
            return false;
        };

        // Rewrite the spine's tail into the loop body: drop the `f(h)` call and
        // the combine, add `param = h` and `acc = acc op t`.
        let acc = self.locals.len() as u32;
        self.locals.push(LocalDecl { ty: Box::new(Ty::I64), is_arg: false, name: None });
        let acc_place = Place { local: acc, proj: Vec::new() };
        let param_place = Place { local: param, proj: Vec::new() };

        let identity = if op == BinOp::Add { 0 } else { 1 };
        self.blocks[call_block as usize].term = Terminator::Goto(combine);
        // The header block is new; the base block becomes the exit.
        let header = self.blocks.len() as u32;
        let body = &mut self.blocks[combine as usize];
        body.stmts.retain(|s| !matches!(s, Stmt::Assign(p, _) if p.proj.is_empty() && p.local == 0));
        body.stmts.push(Stmt::Assign(
            acc_place.clone(),
            Rvalue::Binary(op, Operand::Copy(acc_place.clone()), t_result),
        ));
        body.stmts.push(Stmt::Assign(param_place, Rvalue::Use(h)));
        body.term = Terminator::Goto(header);

        self.blocks[base_block as usize].stmts = vec![Stmt::Assign(
            Place { local: 0, proj: Vec::new() },
            Rvalue::Binary(op, Operand::Copy(acc_place.clone()), guard),
        )];
        self.blocks[base_block as usize].term = Terminator::Goto(ret_block);

        self.blocks.push(BasicBlock {
            stmts: vec![Stmt::Assign(
                Place { local: discr_local, proj: Vec::new() },
                Rvalue::Binary(cmp_op, cmp_a, cmp_b),
            )],
            term: Terminator::SwitchInt {
                discr: Operand::Copy(Place { local: discr_local, proj: Vec::new() }),
                targets: vec![(0, chain_start)],
                otherwise: base_block,
            },
        });

        self.blocks[0].stmts = vec![Stmt::Assign(
            acc_place,
            Rvalue::Use(Operand::Const(Const::Int(identity))),
        )];
        self.blocks[0].term = Terminator::Goto(header);
        true
    }
}

/// Order the blocks so that a branch and its likely target are adjacent, and
/// the target is the fall-through. The back end emits blocks in index order and
/// drops a `goto` to the next block, so a good order removes one taken branch
/// per conditional.
///
/// The fall-through is chosen as the successor that stays inside a loop when
/// there is one. A loop's body is reached from its header every iteration, so
/// it is the hot edge; the exit is taken once. This matters for an early-exit
/// test (`if c { break; }`), where the explicit target is the cold exit, not
/// the body.
pub fn layout_blocks(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.layout_blocks();
    }
}

impl Function {
    fn layout_blocks(&mut self) {
        let n = self.blocks.len();
        if n <= 1 {
            return;
        }
        let succs: Vec<Vec<u32>> = self.blocks.iter().map(|b| successors(&b.term)).collect();
        // `reach[b]` marks every block reachable from `b`. A block on a cycle
        // (it can reach itself) is loop-internal, and any block that reaches one
        // is on the loop's path from the entry.
        let mut reach: Vec<Vec<bool>> = vec![vec![false; n]; n];
        for b in 0..n {
            reach[b][b] = true;
            let mut stack = vec![b];
            while let Some(x) = stack.pop() {
                for &s in &succs[x] {
                    if !reach[b][s as usize] {
                        reach[b][s as usize] = true;
                        stack.push(s as usize);
                    }
                }
            }
        }
        let hot: Vec<bool> = (0..n)
            .map(|b| (0..n).any(|c| c != b && reach[b][c] && reach[c][b]))
            .collect();

        let mut placed = vec![false; n];
        let mut order: Vec<u32> = Vec::with_capacity(n);
        let mut stack: Vec<u32> = vec![0];
        while let Some(start) = stack.pop() {
            if placed[start as usize] {
                continue;
            }
            let mut cur = start;
            loop {
                placed[cur as usize] = true;
                order.push(cur);
                let live: Vec<u32> = succs[cur as usize]
                    .iter()
                    .copied()
                    .filter(|s| !placed[*s as usize])
                    .collect();
                if live.is_empty() {
                    break;
                }
                // The loop-internal edge is the hot one; among those, the
                // branch's taken arm is the likely one (a loop body, not its
                // exit). Otherwise the explicit target of the terminator.
                let taken = explicit_target(&self.blocks[cur as usize].term);
                let next = live
                    .iter()
                    .copied()
                    .find(|s| hot[*s as usize] && Some(*s) == taken)
                    .or_else(|| live.iter().copied().find(|s| hot[*s as usize] && Some(*s) == taken))
                    .or_else(|| live.iter().copied().find(|s| hot[*s as usize]))
                    .or(taken)
                    .filter(|p| live.contains(p))
                    .unwrap_or(live[0]);
                for s in live.iter().rev() {
                    if *s != next {
                        stack.push(*s);
                    }
                }
                cur = next;
            }
        }
        // Any block unreachable from the entry keeps its relative position.
        for b in 0..n as u32 {
            if !placed[b as usize] {
                order.push(b);
            }
        }
        let mut new_index = vec![u32::MAX; n];
        for (i, b) in order.iter().enumerate() {
            new_index[*b as usize] = i as u32;
        }
        let mut kept: Vec<BasicBlock> = Vec::with_capacity(n);
        for b in &order {
            kept.push(std::mem::replace(
                &mut self.blocks[*b as usize],
                BasicBlock { stmts: Vec::new(), term: Terminator::Unreachable },
            ));
        }
        let remap = |t: u32| new_index[t as usize];
        for blk in &mut kept {
            match &mut blk.term {
                Terminator::Goto(t) => *t = remap(*t),
                Terminator::SwitchInt { targets, otherwise, .. } => {
                    for (_, t) in targets.iter_mut() {
                        *t = remap(*t);
                    }
                    *otherwise = remap(*otherwise);
                }
                Terminator::Call { target: Some(t), .. } => *t = remap(*t),
                Terminator::Assert { target, .. } => *target = remap(*target),
                _ => {}
            }
        }
        self.blocks = kept;
    }
}

/// The target a branch is most likely to take: the explicit arm of a
/// conditional, the return block of a call, the target of a `goto`.
fn explicit_target(t: &Terminator) -> Option<u32> {
    match t {
        Terminator::Goto(b) => Some(*b),
        // A `switchInt` over a `bool` (a loop test or an `if`) has one real
        // target and an `otherwise`. rustc puts the branch taken when the
        // condition holds in `otherwise`, and for a loop that is the body, so
        // that is the arm worth falling through to.
        Terminator::SwitchInt { targets, otherwise, .. } => {
            if targets.len() == 1 {
                Some(*otherwise)
            } else {
                targets.first().map(|(_, b)| *b).or(Some(*otherwise))
            }
        }
        Terminator::Call { target, .. } => *target,
        Terminator::Assert { target, .. } => Some(*target),
        _ => None,
    }
}

/// Merge a block into its only successor when that successor has no other
/// predecessor. The two then form one block, so a value defined in the first is
/// visible to the second without a block boundary; the register allocator can
/// keep it in a register instead of spilling it. rustc emits a chain of
/// single-successor blocks for a folded expression, which is exactly the shape
/// that benefits.
pub fn merge_blocks(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.merge_blocks();
    }
}

impl Function {
    fn merge_blocks(&mut self) {
        loop {
            let mut preds: Vec<Vec<u32>> = vec![Vec::new(); self.blocks.len()];
            for (b, blk) in self.blocks.iter().enumerate() {
                for s in successors(&blk.term) {
                    preds[s as usize].push(b as u32);
                }
            }
            let mut merged = false;
            let mut dead: HashSet<u32> = HashSet::new();
            for b in 0..self.blocks.len() {
                if dead.contains(&(b as u32)) {
                    continue;
                }
                let Terminator::Goto(s) = self.blocks[b].term else { continue };
                // Never merge into the entry block (a jump back to bb0 is a loop
                // entry, not a fall-through) or into itself, and only when the
                // successor has exactly this one predecessor.
                if s == 0 || s as usize == b || dead.contains(&s) || preds[s as usize].len() != 1 {
                    continue;
                }
                let tail = std::mem::replace(
                    &mut self.blocks[s as usize],
                    BasicBlock { stmts: Vec::new(), term: Terminator::Unreachable },
                );
                self.blocks[b].stmts.extend(tail.stmts);
                self.blocks[b].term = tail.term;
                dead.insert(s);
                merged = true;
            }
            if !merged {
                break;
            }
            self.compact_blocks(&dead);
        }
    }

    /// Drop the blocks in `dead` and renumber every terminator target.
    fn compact_blocks(&mut self, dead: &HashSet<u32>) {
        let mut new_index: Vec<u32> = vec![u32::MAX; self.blocks.len()];
        let mut kept: Vec<BasicBlock> = Vec::with_capacity(self.blocks.len() - dead.len());
        for (b, blk) in self.blocks.drain(..).enumerate() {
            if dead.contains(&(b as u32)) {
                continue;
            }
            new_index[b] = kept.len() as u32;
            kept.push(blk);
        }
        let remap = |t: u32| new_index[t as usize];
        for blk in &mut kept {
            match &mut blk.term {
                Terminator::Goto(t) => *t = remap(*t),
                Terminator::SwitchInt { targets, otherwise, .. } => {
                    for (_, t) in targets.iter_mut() {
                        *t = remap(*t);
                    }
                    *otherwise = remap(*otherwise);
                }
                Terminator::Call { target: Some(t), .. } => *t = remap(*t),
                Terminator::Assert { target, .. } => *target = remap(*target),
                _ => {}
            }
        }
        self.blocks = kept;
    }
}

/// Collapse `goto`-only blocks. LICM leaves a chain of empty blocks behind when
/// it lifts a hoisted call out of its block, and each one costs a taken jump on
/// every iteration of the loop it sits in. Redirecting the edges straight to the
/// final target removes them; the skipped blocks become unreachable and the back
/// end drops them.
pub fn jump_thread(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.jump_thread();
    }
}

impl Function {
    fn jump_thread(&mut self) {
        // A block worth skipping is empty and jumps unconditionally.
        let is_trampoline = |b: usize| {
            self.blocks[b].stmts.is_empty()
                && matches!(self.blocks[b].term, Terminator::Goto(_))
        };
        // `target[b]` is the first non-trampoline block reached from `b`.
        let mut target: Vec<u32> = (0..self.blocks.len() as u32).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for b in 0..self.blocks.len() {
                if !is_trampoline(b) {
                    continue;
                }
                if let Terminator::Goto(t) = self.blocks[b].term {
                    let resolved = target[t as usize];
                    if target[b] != resolved {
                        target[b] = resolved;
                        changed = true;
                    }
                }
            }
        }
        let resolve = |t: u32| target[t as usize];
        for b in &mut self.blocks {
            match &mut b.term {
                Terminator::Goto(t) => *t = resolve(*t),
                Terminator::SwitchInt { targets, otherwise, .. } => {
                    for (_, t) in targets.iter_mut() {
                        *t = resolve(*t);
                    }
                    *otherwise = resolve(*otherwise);
                }
                Terminator::Call { target: Some(t), .. } => *t = resolve(*t),
                _ => {}
            }
        }
    }
}

/// Run loop-invariant code motion over the whole crate, to a fixpoint: hoisting
/// a binding can make the loads that read through it invariant on the next
/// round.
pub fn licm(prog: &mut Program) {
    for _ in 0..LICM_ROUNDS {
        let mut changed = false;
        for f in &mut prog.funcs {
            if f.licm_once() {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

impl Function {
    /// Hoist loop-invariant computations out of each natural loop.
    ///
    /// Only a *backward* branch is treated as a loop (its header is the target
    /// and its latches are the blocks that jump back). The pass hoists a
    /// statement to the header's sole predecessor — the preheader, which rustc
    /// emits for every loop — and only when:
    ///
    /// * the statement is a pure call or a scalar arithmetic rvalue;
    /// * every operand it reads is defined *outside* the loop and never
    ///   redefined inside it (so no value in the loop can be a `Move` that the
    ///   hoisted statement would then read as already-consumed);
    /// * the loop writes no memory at all, and every call in it is to a pure
    ///   function (a call is a barrier: it can read or write anything);
    /// * every destination is written exactly once in the whole function, so the
    ///   hoisted definition dominates all its uses and no second definition can
    ///   clobber it.
    ///
    /// The preheader is the loop header's only predecessor. Hoisting to it is
    /// safe because it is entered on the way to the loop and executed once.
    fn licm_once(&mut self) -> bool {
        let nblocks = self.blocks.len();
        if nblocks == 0 {
            return false;
        }
        let defs = self.def_counts();
        let use_counts = self.use_counts();

        // Which blocks write memory (a store, a call, or an address-taking op).
        let writes_mem: Vec<bool> = self.blocks.iter().map(|b| block_writes_mem(b)).collect();
        let impure_call: Vec<bool> = self.blocks.iter().map(|b| block_has_impure_call(b)).collect();

        // Predecessors, from the terminator successors.
        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); nblocks];
        for (b, blk) in self.blocks.iter().enumerate() {
            for s in successors(&blk.term) {
                if let Some(p) = preds.get_mut(s as usize) {
                    p.push(b as u32);
                }
            }
        }

        // Each back edge (an edge `p -> s` where `s` can reach `p`) is a latch;
        // group latches by their header. Numeric order is not enough: rustc can
        // lay a loop out so its preheader has a *lower* block number than the
        // header, which would misread that edge as a back edge.
        let mut latches: HashMap<u32, Vec<u32>> = HashMap::new();
        for (b, blk) in self.blocks.iter().enumerate() {
            for s in successors(&blk.term) {
                if s == b as u32 || reaches(&preds, s, b as u32) {
                    latches.entry(s).or_default().push(b as u32);
                }
            }
        }
        // Deterministic order so the output is stable across runs.
        let mut headers: Vec<u32> = latches.keys().copied().collect();
        headers.sort_unstable();

        let mut any_changed = false;
        for header in headers {
            if self.licm_loop(header, &latches, &preds, &defs, &use_counts, &writes_mem, &impure_call) {
                any_changed = true;
            }
        }
        any_changed
    }

    #[allow(clippy::too_many_arguments)]
    fn licm_loop(
        &mut self,
        header: u32,
        latches: &HashMap<u32, Vec<u32>>,
        preds: &[Vec<u32>],
        defs: &HashMap<u32, u32>,
        use_counts: &HashMap<u32, u32>,
        writes_mem: &[bool],
        impure_call: &[bool],
    ) -> bool {
        // The preheader is the header's only predecessor from *outside* the
        // loop: then the loop is entered from exactly one place, and hoisting to
        // it runs once. (The latches are predecessors too, but they are inside.)
        let loop_blocks = self.loop_blocks(header, latches);
        let mut outside: Vec<u32> = preds[header as usize]
            .iter()
            .copied()
            .filter(|p| !loop_blocks.contains(p))
            .collect();
        if outside.len() != 1 {
            return false;
        }
        let preheader = outside.pop().unwrap();
        if preheader == header {
            return false;
        }
        // Redirecting the preheader into the hoisted-call chain replaces its
        // terminator, so it must be a plain `goto` to the header; a conditional
        // preheader would lose its other edge.
        if !matches!(self.blocks[preheader as usize].term, Terminator::Goto(t) if t == header) {
            return false;
        }
        // A loop that writes memory, or calls anything impure, gives up: a call
        // can read or write state a hoisted load would have to observe.
        if loop_blocks
            .iter()
            .any(|b| writes_mem[*b as usize] || impure_call[*b as usize])
        {
            return false;
        }
        // Values defined inside the loop cannot be read outside it. Call
        // destinations are terminators, not statements, so they are collected
        // separately: a hoisted statement must never read one, because the
        // hoisted call is emitted after the hoisted statements.
        let term_defs = |blk: &BasicBlock| -> Option<u32> {
            match &blk.term {
                Terminator::Call { dest, .. } if dest.proj.is_empty() => Some(dest.local),
                _ => None,
            }
        };
        let defined_in: HashSet<u32> = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(b, _)| loop_blocks.contains(&(*b as u32)))
            .flat_map(|(_, blk)| {
                blk.stmts
                    .iter()
                    .filter_map(written_local)
                    .chain(term_defs(blk))
            })
            .collect();
        // A `Move` is not an invalidation here: the back end copies the value,
        // so a `move`d scalar still holds it. Excluding moved locals would block
        // hoisting the very loads the inliner creates, because rustc emits the
        // accumulator fold over them with `move`.

        // `_0` (the return slot) is written by `return` and by calls; never hoist
        // a definition of it, and never treat it as invariant.
        let is_invariant = |l: u32| !defined_in.contains(&l) && l != 0;

        // A deterministic block order, so the hoisted statements come out in
        // the same order on every run.
        let mut ordered: Vec<u32> = loop_blocks.iter().copied().collect();
        ordered.sort_unstable();

        // Collect hoistable statements, in block order.
        let mut hoisted: Vec<Stmt> = Vec::new();
        for &b in &ordered {
            let blk = &self.blocks[b as usize];
            for s in &blk.stmts {
                let Stmt::Assign(dest, rv) = s else { continue };
                if !dest.proj.is_empty() {
                    continue;
                }
                // The destination must be single-definition in the whole
                // function, so hoisting dominates its uses and nothing else
                // writes it.
                if defs.get(&dest.local).copied().unwrap_or(0) != 1 {
                    continue;
                }
                // A dead destination (no uses) is not worth hoisting.
                if use_counts.get(&dest.local).copied().unwrap_or(0) == 0 {
                    continue;
                }
                if !hoistable_rv(rv) {
                    continue;
                }
                if !rv_operands(rv).iter().all(|p| is_invariant(p.local)) {
                    continue;
                }
                hoisted.push(s.clone());
            }
        }

        // Collect hoistable calls. A pure call with invariant arguments produces
        // the same value every iteration, so it runs once in the preheader
        // instead. rustc keeps the accessor calls in `refs`' hot loop; hoisting
        // them is what the AST backend's `licm` achieves by inlining first.
        let mut hoisted_calls: Vec<(u32, u32, CallTarget, Vec<Operand>, u32)> = Vec::new();
        for &b in &ordered {
            if b == header {
                continue; // the header's terminator is the loop test
            }
            let blk = &self.blocks[b as usize];
            let Terminator::Call { dest, func, args, target: Some(t) } = &blk.term else {
                continue;
            };
            if !dest.proj.is_empty() || dest.local == 0 {
                continue;
            }
            if defs.get(&dest.local).copied().unwrap_or(0) != 1 {
                continue;
            }
            let CallTarget::Direct(name) = func else { continue };
            if !pure_call(name) {
                continue;
            }
            if !call_args_invariant(args, &is_invariant) {
                continue;
            }
            hoisted_calls.push((b, dest.local, func.clone(), args.clone(), *t));
        }

        if hoisted.is_empty() && hoisted_calls.is_empty() {
            return false;
        }

        // Remove the hoisted statements from their blocks. A statement is
        // identified by its destination local, which the single-definition test
        // already made unique.
        let moved: HashSet<u32> = hoisted.iter().filter_map(written_local).collect();
        if !moved.is_empty() {
            for b in &loop_blocks {
                let blk = &mut self.blocks[*b as usize];
                blk.stmts.retain(|s| match written_local(s) {
                    Some(l) => !moved.contains(&l),
                    None => true,
                });
            }
        }
        // A hoisted call leaves a plain `goto` behind, keeping the loop's
        // control flow intact.
        for (b, _, _, _, t) in &hoisted_calls {
            self.blocks[*b as usize].term = Terminator::Goto(*t);
        }

        // Append a chain of blocks holding the hoisted calls, in their original
        // order, ending at the loop header, and route the preheader into it.
        let mut chain_entry = header;
        if !hoisted_calls.is_empty() {
            let mut blocks: Vec<BasicBlock> = Vec::with_capacity(hoisted_calls.len());
            for i in 0..hoisted_calls.len() {
                let next = if i + 1 < hoisted_calls.len() {
                    (self.blocks.len() + i + 1) as u32
                } else {
                    header
                };
                let (_, dest_local, func, args, _) = &hoisted_calls[i];
                blocks.push(BasicBlock {
                    stmts: Vec::new(),
                    term: Terminator::Call {
                        dest: Place { local: *dest_local, proj: Vec::new() },
                        func: func.clone(),
                        args: args.clone(),
                        target: Some(next),
                    },
                });
            }
            chain_entry = self.blocks.len() as u32;
            self.blocks.extend(blocks);
        }

        // Prepend the hoisted statements to the preheader and redirect it into
        // the call chain (or straight to the header when there is none).
        // The hoisted statements are appended to the preheader, *after* its own
        // statements: the loop-invariant operands they read are defined there
        // (rustc emits the loop setup in the preheader), so prepending would
        // read them before they are written.
        let pre = &mut self.blocks[preheader as usize];
        pre.stmts.extend(hoisted);
        pre.term = Terminator::Goto(chain_entry);
        true
    }

    /// The set of blocks that make up the loop with the given header: every
    /// block that can reach a latch without passing through the header, plus
    /// the header itself.
    fn loop_blocks(&self, header: u32, latches: &HashMap<u32, Vec<u32>>) -> HashSet<u32> {
        let mut in_loop: HashSet<u32> = HashSet::new();
        in_loop.insert(header);
        let mut worklist: Vec<u32> = latches.get(&header).cloned().unwrap_or_default();
        while let Some(b) = worklist.pop() {
            if !in_loop.insert(b) {
                continue;
            }
            // Walk predecessors, stopping at the header (its other predecessors
            // are outside the loop).
            for (p, blk) in self.blocks.iter().enumerate() {
                let _ = blk;
                if successors(&self.blocks[p].term).contains(&b) && p as u32 != header {
                    worklist.push(p as u32);
                }
            }
        }
        in_loop
    }
}

/// Whether `from` can reach `to` by following predecessor edges, i.e. `from`
/// dominates-or-equals `to` in the block order. Used to tell a back edge from a
/// forward one without relying on block numbering.
fn reaches(preds: &[Vec<u32>], from: u32, to: u32) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![to];
    while let Some(b) = stack.pop() {
        if b == from {
            return true;
        }
        if !seen.insert(b) {
            continue;
        }
        for &p in &preds[b as usize] {
            stack.push(p);
        }
    }
    false
}

/// The successor blocks of a terminator.
pub(crate) fn successors(t: &Terminator) -> Vec<u32> {
    match t {
        Terminator::Goto(b) => vec![*b],
        Terminator::SwitchInt { targets, otherwise, .. } => {
            let mut v: Vec<u32> = targets.iter().map(|(_, b)| *b).collect();
            v.push(*otherwise);
            v
        }
        Terminator::Call { target, .. } => target.iter().copied().collect(),
        Terminator::Assert { target, .. } => vec![*target],
        Terminator::Return | Terminator::Unreachable | Terminator::Abort => Vec::new(),
    }
}

/// True when a call's arguments are all invariant: each is a constant, or a
/// read of a whole local that the loop neither defines nor moves.
fn call_args_invariant(args: &[Operand], is_invariant: &impl Fn(u32) -> bool) -> bool {
    args.iter().all(|a| match a {
        Operand::Const(_) => true,
        Operand::Copy(p) | Operand::Move(p) => p.proj.is_empty() && is_invariant(p.local),
    })
}

/// True when a block contains anything that could write memory: a store through
/// a projection (`*p = x`, `arr[i] = x`), an address-taking rvalue, or a call to
/// a function that is not known pure. A pure call is not a store, so it does not
/// block hoisting a load past it.
fn block_writes_mem(b: &BasicBlock) -> bool {
    b.stmts.iter().any(|s| match s {
        Stmt::Assign(dest, rv) => {
            !dest.proj.is_empty()
                || matches!(
                    rv,
                    Rvalue::Ref { .. }
                        | Rvalue::RawRef { .. }
                        | Rvalue::Array(_)
                        | Rvalue::Aggregate(_)
                        | Rvalue::Repeat { .. }
                )
        }
        Stmt::Nop => false,
    }) || block_has_impure_call(b)
}

/// True when a block contains a call that is not to a known-pure function.
fn block_has_impure_call(b: &BasicBlock) -> bool {
    match &b.term {
        Terminator::Call { func: CallTarget::Direct(name), .. } => !pure_call(name),
        Terminator::Call { func: CallTarget::Indirect, .. } => true,
        _ => false,
    }
}

/// True when an rvalue is cheap and pure enough to hoist: a load or copy, a
/// scalar arithmetic op, or a cast. A copy of an invariant value is hoisted too,
/// because it is often the binding of an inlined call's argument: hoisting it
/// first is what makes the loads that read through it invariant on a later
/// round.
fn hoistable_rv(rv: &Rvalue) -> bool {
    match rv {
        Rvalue::Use(_) => true,
        Rvalue::Binary(op, _, _) => !matches!(op, BinOp::Div | BinOp::Rem),
        Rvalue::Unary(_, _) => true,
        Rvalue::Cast { .. } => true,
        Rvalue::Discriminant(_) => false,
        _ => false,
    }
}

/// The locals a statement reads. Used to test invariance. A call is handled by
/// the block-level purity rule, so only its arguments matter here.
fn rv_operands(rv: &Rvalue) -> Vec<Place> {
    let mut v = Vec::new();
    let mut push = |op: &Operand| {
        if let Operand::Copy(p) | Operand::Move(p) = op {
            v.push(p.clone());
        }
    };
    match rv {
        Rvalue::Use(op) => push(op),
        Rvalue::Binary(_, a, b) => {
            push(a);
            push(b);
        }
        Rvalue::Unary(_, a) => push(a),
        Rvalue::Cast { op, .. } => push(op),
        Rvalue::Repeat { op, .. } => push(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => ops.iter().for_each(|op| push(op)),
        Rvalue::WithOverflow { a, b, .. } => {
            push(a);
            push(b);
        }
        _ => {}
    }
    v
}
