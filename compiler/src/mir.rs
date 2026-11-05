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

use std::collections::HashMap;

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
    /// `allocN` static data blobs (string literals), by allocation id.
    pub allocs: HashMap<u32, Vec<u8>>,
    /// `const NAME: TY = ...;` items at the crate scope.
    pub consts: HashMap<String, i64>,
}

impl Program {
    pub fn func(&self, name: &str) -> Option<&Function> {
        self.funcs.iter().find(|f| f.name == name)
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
        } else if l.starts_with("alloc") && l.contains("(size:") {
            let (id, bytes, next) = parse_alloc(&lines, i)?;
            prog.allocs.insert(id, bytes);
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
fn parse_alloc(lines: &[String], start: usize) -> Result<(u32, Vec<u8>, usize), String> {
    let header = lines[start].trim();
    let id: u32 = header
        .strip_prefix("alloc")
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("mir: bad alloc header `{header}`"))?;
    let mut bytes = Vec::new();
    let mut i = start + 1;
    while i < lines.len() {
        let l = lines[i].trim();
        if l.starts_with('}') {
            return Ok((id, bytes, i + 1));
        }
        // A row is `68 65 6c ...` optionally followed by `│ ascii`. Split at
        // the box-drawing character, then read hex pairs.
        let hex = l.split('│').next().unwrap_or(l);
        for tok in hex.split_whitespace() {
            if let Ok(b) = u8::from_str_radix(tok, 16) {
                bytes.push(b);
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
    let func = if func.contains("::") || func.starts_with('<') || func.contains(" as ") {
        CallTarget::Indirect
    } else {
        CallTarget::Direct(func)
    };
    Some(Terminator::Call { dest, func, args, target })
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
        return Some(Terminator::Abort);
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

/// Split at a top-level ` as ` (for a cast rvalue).
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
