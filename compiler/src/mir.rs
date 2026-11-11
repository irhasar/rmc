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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// A resolved type, in the narrow sense the backend needs.
///
/// Every type is either a scalar (lives in one 64-bit register) or an aggregate
/// (lives in memory, named by its address). `Ref`/`RawPtr` are scalars: a
/// pointer is one word, and a dereference reads through it.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    I64,
    Bool,
    /// An integer narrower than 64 bits, or a 64-bit integer whose
    /// signedness differs from `i64`. The width and signedness matter to the
    /// exact op: `u64 >> n` fills with zeros, `u32` division is unsigned, and
    /// `as` must truncate to the right width and extend with the right sign.
    /// `i64` keeps its own variant (the common case, so the hot paths match on
    /// one tag) and `Int` covers everything else.
    Int {
        /// 1, 2, 4 or 8 bytes.
        width: u8,
        /// `false` for `u*`/`usize`, `true` for `i*`/`isize`.
        signed: bool,
    },
    /// A one-byte integer (`u8`/`i8`), kept apart from `Int` because pointer
    /// arithmetic scales by the pointee's size: `Offset` over a `*const u8`
    /// must advance one byte, not eight. `i8` is sign-extended on load, `u8`
    /// zero-extended.
    U8,
    Ptr(TyBox),
    /// A reference or raw pointer; the pointee type is only used to size a
    /// `&raw` / `*p` load.
    Ref(TyBox),
    Struct(String),
    Array(TyBox, u64),
    Tuple(Vec<TyBox>),
    /// `()` and any type the backend treats as zero-sized.
    Unit,
    /// `f64`: one 64-bit word holding an IEEE-754 double.
    F64,
    /// `f32`: one 32-bit word holding an IEEE-754 single, stored in the low
    /// half of a 64-bit slot.
    F32,
    /// A `&str` literal or an unmodelled type; treated as a scalar.
    Other,
}

impl Ty {
    /// A signed integer of the given byte width (1, 2, 4 or 8).
    pub fn signed_int(width: u8) -> Ty {
        if width == 8 {
            Ty::I64
        } else {
            Ty::Int { width, signed: true }
        }
    }

    /// An unsigned integer of the given byte width (1, 2, 4 or 8).
    pub fn unsigned_int(width: u8) -> Ty {
        if width == 1 {
            // A `u8` has its own variant so a byte load/store is chosen.
            Ty::U8
        } else {
            Ty::Int { width, signed: false }
        }
    }

    /// Whether this is an integer type (any width, either signedness).
    pub fn is_int(&self) -> bool {
        matches!(self, Ty::I64 | Ty::Int { .. } | Ty::U8)
    }

    /// `(width_in_bytes, signed)` for an integer, or `None` for anything else.
    pub fn int_width_sign(&self) -> Option<(u8, bool)> {
        match self {
            Ty::I64 => Some((8, true)),
            Ty::U8 => Some((1, false)),
            Ty::Int { width, signed } => Some((*width, *signed)),
            _ => None,
        }
    }

    /// Whether an integer type is unsigned. `false` for non-integers.
    pub fn is_unsigned(&self) -> bool {
        self.int_width_sign().map_or(false, |(_, s)| !s)
    }
}

pub type TyBox = Box<Ty>;

impl Ty {
    pub fn is_aggregate(&self) -> bool {
        matches!(self, Ty::Struct(_) | Ty::Array(..) | Ty::Tuple(_))
    }

    /// True for an IEEE-754 float. A float lives in a general-purpose register
    /// like any other scalar (the back end moves it through `%rax` and SSE
    /// registers), so most of the integer plumbing treats it as one word; only
    /// arithmetic, comparison, casts and formatting branch on this.
    pub fn is_float(&self) -> bool {
        matches!(self, Ty::F32 | Ty::F64)
    }
}

/// One operand of a statement or terminator.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Const(Const),
    /// `<T as Trait>::fmt` — a reified method. `ty_name` and `trait_name`
    /// identify which runtime formatter the back end must materialise the
    /// address of; the MIR path never calls it indirectly. The type is kept as
    /// its source text so width and signedness (`i32` vs `u64`) survive.
    Fmt {
        ty_name: String,
        trait_name: String,
    },
}

/// A constant operand.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(i64),
    Bool(bool),
    /// An `f64`/`f32` literal (`const 5.5f64`, `const 2f64`), held as its bit
    /// pattern so the back end can materialise it without a parser dependency
    /// on Rust's decimal-to-binary rounding. The width says which one it is.
    Float(f64, bool),
    /// A `char` literal (`'A'`), held as its Unicode scalar value. Rust's
    /// `char` is a 32-bit scalar, so the formatter reads four bytes.
    Char(u32),
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
    /// `b"..."` — a byte-string literal. `format_args!` lowers its template to
    /// one of these, so the MIR path must carry the bytes rather than treat the
    /// literal as text.
    Bytes(Vec<u8>),
    /// A zero-sized value (`PhantomData`). It occupies no space, so an
    /// aggregate that lists one must not reserve a word for it.
    Zst,
    /// A struct, tuple, array or enum-variant constant the MIR printer spells
    /// out inline (`const P {{ x: 1_i64, y: 2_i64 }}`, `const E::A`). Its bytes
    /// are materialised in `.rodata` at codegen time.
    Agg {
        name: String,
        ops: Vec<Operand>,
    },
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
    /// `(x as Variant)` — the payload of an enum variant, e.g. `(_1 as B).0`.
    /// The variant name lets the backend find the payload's offset within the
    /// enum's tagged-union layout.
    Variant(String),
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
    /// `Enum::Variant { .. }` — build an enum variant. The name is `Enum::Variant`.
    Enum { name: String, ops: Vec<Operand> },
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
    /// A bit-preserving reinterpretation (`f64::to_bits`, `u64::from`), which
    /// must copy the bits, not convert the value.
    Transmute,
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
    /// `drop(_N) -> [return: bbX, ...]`: run `_N`'s destructor, then continue.
    /// The back end calls the type's `Drop::drop` impl with the value's
    /// address. Folding this to a plain `goto` (as an earlier revision did)
    /// silently skipped every destructor.
    Drop { place: Place, target: u32 },
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

/// A promoted constant: `const main::promoted[N]: &T = { _1 = const V; _0 =
/// &_1; return; }`. rustc promotes a literal used behind a reference (a format
/// argument, a `&[1, 2, 3]`, ...) into a hidden static, and MIR refers to it by
/// name (`const main::promoted[0]`). The body is reduced to the value it
/// returns the address of, so the back end can materialise it in `.rodata`.
#[derive(Debug, Clone)]
pub struct Promoted {
    /// The referent's byte size; the `.rodata` blob is padded to a word.
    pub size: i64,
    /// `(byte offset, value)` for each scalar written into the blob.
    pub bytes: Vec<(i64, i64)>,
    /// When the referent is itself a `&str`, the string literal it points at.
    /// `&&str` is what rustc promotes for `"a" == "a"` and `match s { "x" => }`:
    /// the outer pointer addresses a word that holds the address of the string
    /// record, so the blob word is a relocation, not a scalar.
    pub str_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub funcs: Vec<Function>,
    /// `allocN` static data blobs (string literals, promoted constants,
    /// `static` data), by allocation id.
    pub allocs: HashMap<u32, AllocData>,
    /// `const NAME: TY = ...;` items at the crate scope.
    pub consts: HashMap<String, i64>,
    /// `main::promoted[N]` -> the value it points at, for the ones the back end
    /// can fold. A name that is absent is rejected rather than miscompiled.
    pub promoted: HashMap<String, Promoted>,
    /// Receiver type name -> the emitted symbol of its `Drop::drop` impl. The
    /// MIR prints each impl's body as `fn <impl at ...>::drop(_1: &mut T)`, so
    /// every impl normalises to the same `drop` name; renaming each to
    /// `__rmc_drop_<T>` keeps them distinct, and the map lets the back end
    /// resolve a `drop(_N)` terminator from `_N`'s type.
    pub drop_impls: HashMap<String, String>,
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
        // A local read as a dynamic index (`_1[_7]`) must keep its definition:
        // folding it away leaves the index slot with no writer, so the store
        // addresses element 0 (`arr[i] = x` wrote the wrong element).
        let index_locals: HashSet<u32> = self
            .blocks
            .iter()
            .flat_map(|b| {
                let stmts = b.stmts.iter().flat_map(|s| match s {
                    Stmt::Assign(d, rv) => {
                        let mut v: Vec<u32> = place_indices(d);
                        scan_rv_operands(rv, &mut |op| {
                            if let Operand::Copy(p) | Operand::Move(p) = op {
                                v.extend(place_indices(p));
                            }
                        });
                        v
                    }
                    _ => Vec::new(),
                });
                let term = std::iter::once(&b.term).flat_map(|t| match t {
                    Terminator::SwitchInt { discr, .. } => operand_indices(discr),
                    Terminator::Assert { cond, .. } => operand_indices(cond),
                    Terminator::Drop { place, .. } => place_indices(place),
                    Terminator::Call { args, .. } => {
                        args.iter().flat_map(operand_indices).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                });
                stmts.chain(term)
            })
            .collect();
        let subst_local: HashSet<u32> = (0..self.locals.len() as u32)
            .filter(|l| self.safe_subst(*l, &defs) && !index_locals.contains(l))
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
        let read_op = |op: &Operand, uses: &mut HashMap<u32, u32>| {
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
                Terminator::Drop { place, .. } => {
                    *uses.entry(place.local).or_insert(0) += 1;
                }
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
        Rvalue::Use(op) => {
            // A place used as an operand (`_1[_7]`) reads its index locals as
            // well as its base. Missing them would let a dead-copy pass drop the
            // index's definition and leave a stale slot behind.
            if let Operand::Copy(p) | Operand::Move(p) = op {
                for proj in &p.proj {
                    if let Proj::IndexLocal(i) = proj {
                        f(&Operand::Copy(Place { local: *i, proj: Vec::new() }));
                    }
                }
            }
            f(op);
        }
        Rvalue::Binary(_, a, b) => {
            f(a);
            f(b);
        }
        Rvalue::Unary(_, a) => f(a),
        Rvalue::Cast { op, .. } => f(op),
        Rvalue::Repeat { op, .. } => f(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => ops.iter().for_each(|op| f(op)),
        Rvalue::Enum { ops, .. } => ops.iter().for_each(|op| f(op)),
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

/// Call `f` for every operand a statement reads, mutably.
fn for_each_operand_mut(rv: &mut Rvalue, f: &mut impl FnMut(&mut Operand)) {
    match rv {
        Rvalue::Use(op) => f(op),
        Rvalue::Binary(_, a, b) => {
            f(a);
            f(b);
        }
        Rvalue::Unary(_, a) => f(a),
        Rvalue::Cast { op, .. } => f(op),
        Rvalue::Repeat { op, .. } => f(op),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) => ops.iter_mut().for_each(|op| f(op)),
        Rvalue::Enum { ops, .. } => ops.iter_mut().for_each(|op| f(op)),
        Rvalue::WithOverflow { a, b, .. } => {
            f(a);
            f(b);
        }
        _ => {}
    }
}

/// The locals a place reads as a dynamic index (`_1[_7]` reads `_7`).
fn place_indices(p: &Place) -> Vec<u32> {
    p.proj
        .iter()
        .filter_map(|proj| match proj {
            Proj::IndexLocal(i) => Some(*i),
            _ => None,
        })
        .collect()
}

/// The locals an operand reads as a dynamic index.
fn operand_indices(op: &Operand) -> Vec<u32> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => place_indices(p),
        _ => Vec::new(),
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
        rewrite_place(p, subst);
    }
}

/// Rewrite a place read through the alias map: its index locals (`_1[_7]`), and
/// its base local when the alias has no projection of its own. A `Ref`/`RawRef`
/// target is rewritten too, but only when the alias is a plain local: aliasing a
/// projection would change which address is taken.
fn rewrite_place(p: &mut Place, subst: &HashMap<u32, Place>) {
    rewrite_place_indices(p, subst);
    if let Some(src) = subst.get(&p.local) {
        if p.proj.is_empty() || src.proj.is_empty() {
            let proj = std::mem::take(&mut p.proj);
            *p = src.clone();
            p.proj.extend(proj);
        }
    }
}

/// Rewrite only the dynamic index locals of a place (`_1[_7]`), leaving its
/// base local alone.
///
/// This is what an assignment's destination needs. The base names the storage
/// that is written, and `_d = copy _x` gives `_d` its own storage, so
/// `_d.0 = 5` must not be redirected to `_x.0`. The index, by contrast, is an
/// address computation: if `_7`'s definition was folded into `_9`, the index
/// must follow the alias or the store addresses the wrong element.
fn rewrite_place_indices(p: &mut Place, subst: &HashMap<u32, Place>) {
    for proj in &mut p.proj {
        if let Proj::IndexLocal(i) = proj {
            if let Some(src) = subst.get(i) {
                if src.proj.is_empty() {
                    *i = src.local;
                }
            }
        }
    }
}

fn rewrite_stmt(s: &mut Stmt, subst: &HashMap<u32, Place>) {
    if subst.is_empty() {
        return;
    }
    let Stmt::Assign(dest, rv) = s else { return };
    // The destination is a place too: `_2[_9] = v` indexes through `_9`, and if
    // `_9`'s definition was folded away the store would address the wrong
    // element. Rewriting only the rvalue left the index dangling. The base
    // local is left alone: the destination names the storage that is written.
    rewrite_place_indices(dest, subst);
    match rv {
        Rvalue::Use(op) => rewrite_op(op, subst),
        Rvalue::Binary(_, a, b) => {
            rewrite_op(a, subst);
            rewrite_op(b, subst);
        }
        Rvalue::Unary(_, a) => rewrite_op(a, subst),
        Rvalue::Cast { op, .. } => rewrite_op(op, subst),
        Rvalue::Repeat { op, .. } => rewrite_op(op, subst),
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
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
            rewrite_place(place, subst);
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
        Terminator::Drop { place, .. } => rewrite_place(place, subst),
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
    let mut prog = Program {
        funcs: Vec::new(),
        allocs: HashMap::new(),
        consts: HashMap::new(),
        promoted: HashMap::new(),
        drop_impls: HashMap::new(),
    };
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i].trim();
        if l.is_empty() {
            i += 1;
            continue;
        }
        if l.starts_with("fn ") {
            let (f, next) = parse_fn(&lines, i)?;
            // Record a `Drop` impl's receiver type -> its renamed symbol, so the
            // back end can resolve a `drop(_N)` terminator to the impl.
            if l.contains("<impl") {
                if let Some((open, close)) = l.find('(').zip(l.find(')')) {
                    let head = l[..open].trim();
                    let arg_tys = parse_arg_tys(&l[open + 1..close]).unwrap_or_default();
                    if head.rsplit(">::").next().map(str::trim) == Some("drop") {
                        if let Some(recv) = method_receiver_ty(&arg_tys) {
                            prog.drop_impls.insert(recv.clone(), format!("__rmc_drop_{recv}"));
                        }
                    }
                }
            }
            prog.funcs.push(f);
            i = next;
        } else if l.starts_with("const ") && l.contains("::promoted[") {
            let (name, p, next) = parse_promoted(&lines, i, &prog.consts)?;
            // A promoted constant the parser cannot fold is left out of the map,
            // so a use of it is a clear back-end error rather than a wrong value.
            if let Some(p) = p {
                prog.promoted.insert(name, p);
            }
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
    resolve_impl_calls(&mut prog);
    canonicalize_types(&mut prog);
    Ok(prog)
}

/// Reconcile the several names rustc's MIR printer uses for one type. A type is
/// spelled with a path *relative* to the item that mentions it: `main` prints
/// `a::A`, but the impl inside module `a` prints the same type as `A`. Two
/// distinct types can also share a last segment (`a::A` and `b::A`) or a whole
/// tail (`a::A` and `outer::a::A`). `parse_ty` keeps every qualified path whole;
/// this pass then lets a *bare* name adopt the one qualified name it is a
/// `::`-suffix of, so an impl definition, its call sites and the value's own
/// layout all key on one string. A name that already carries a path is a
/// complete spelling and never adopts a longer path it merely ends with — a
/// `::`-suffix is not proof of identity, and merging `a::A` into `outer::a::A`
/// would give two distinct types one layout and one symbol. A bare name with no
/// unique longer match is left alone.
fn canonicalize_types(prog: &mut Program) {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for f in &prog.funcs {
        for l in &f.locals {
            collect_struct_names(&l.ty, &mut names);
        }
        for b in &f.blocks {
            for s in &b.stmts {
                collect_stmt_tys(s, &mut names);
            }
        }
    }
    for k in prog.drop_impls.keys() {
        names.insert(k.clone());
    }

    let mut canon: BTreeMap<String, String> = BTreeMap::new();
    for n in &names {
        canon.insert(n.clone(), n.clone());
    }
    // Each name adopts the unique *longest* name it is a `::`-suffix of. When
    // two candidates tie for longest (`A` seen as both `a::A` and `b::A`) the
    // name is genuinely ambiguous and is left alone.
    for n in &names {
        // Only a bare name adopts a qualified one: the printer spells a type
        // relative to the item that mentions it, so a lone `A` may stand for
        // `a::A`. A name that already carries a path is a complete spelling and
        // must not absorb a longer path it merely ends with — `a::A` is a
        // `::`-suffix of `outer::a::A`, a different type, and merging them makes
        // one impl's body win both calls.
        if n.contains("::") {
            continue;
        }
        let suffix = format!("::{n}");
        let mut best: Option<&String> = None;
        let mut tied = false;
        for q in &names {
            if q != n && q.ends_with(&suffix) {
                match best {
                    Some(b) if q.len() == b.len() => tied = true,
                    Some(b) if q.len() < b.len() => {}
                    _ => {
                        best = Some(q);
                        tied = false;
                    }
                }
            }
        }
        if let Some(b) = best.filter(|_| !tied) {
            canon.insert(n.clone(), b.clone());
        }
    }

    for f in &mut prog.funcs {
        for l in &mut f.locals {
            rename_struct_names(&mut l.ty, &canon);
        }
        for b in &mut f.blocks {
            for s in &mut b.stmts {
                rename_stmt_tys(s, &canon);
            }
            if let Terminator::Call { func: CallTarget::Direct(name), .. } = &mut b.term {
                if let Some((recv, rest)) = name
                    .split_once("__")
                    .filter(|(k, _)| canon.contains_key(*k))
                    .map(|(k, r)| (k.to_string(), r.to_string()))
                {
                    *name = format!("{}__{rest}", canon[&recv]);
                }
            }
        }
    }
    // An impl method's symbol is `<receiver>__<method>`; the receiver is the
    // long-name form the definition happened to print, so rename the prefix.
    let mut keys: Vec<&String> = canon.iter().filter(|(k, v)| k != v).map(|(k, _)| k).collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    for f in &mut prog.funcs {
        if let Some((recv, rest)) = keys
            .iter()
            .find_map(|k| f.name.strip_prefix(k.as_str()).and_then(|r| r.strip_prefix("__")).map(|r| (*k, r)))
        {
            f.name = format!("{}__{rest}", canon[recv]);
        }
    }
    let mut new_drops: HashMap<String, String> = HashMap::new();
    for (recv, sym) in &prog.drop_impls {
        let c = canon.get(recv).cloned().unwrap_or_else(|| recv.clone());
        let sym = sym
            .strip_prefix("__rmc_drop_")
            .map(|r| format!("__rmc_drop_{}", canon.get(r).cloned().unwrap_or_else(|| r.to_string())))
            .unwrap_or_else(|| sym.clone());
        new_drops.insert(c, sym);
    }
    prog.drop_impls = new_drops;
}

/// Insert every `Ty::Struct` name reachable from `ty`.
fn collect_struct_names(ty: &Ty, out: &mut BTreeSet<String>) {
    match ty {
        Ty::Struct(n) => {
            out.insert(n.clone());
        }
        Ty::Ref(t) | Ty::Ptr(t) | Ty::Array(t, _) => collect_struct_names(t, out),
        Ty::Tuple(ts) => ts.iter().for_each(|t| collect_struct_names(t, out)),
        _ => {}
    }
}

/// Rewrite every `Ty::Struct` name reachable from `ty` through `canon`.
fn rename_struct_names(ty: &mut Ty, canon: &BTreeMap<String, String>) {
    match ty {
        Ty::Struct(n) => {
            if let Some(c) = canon.get(n) {
                *n = c.clone();
            }
        }
        Ty::Ref(t) | Ty::Ptr(t) | Ty::Array(t, _) => rename_struct_names(t, canon),
        Ty::Tuple(ts) => ts.iter_mut().for_each(|t| rename_struct_names(t, canon)),
        _ => {}
    }
}

fn collect_stmt_tys(s: &Stmt, out: &mut BTreeSet<String>) {
    if let Stmt::Assign(_, rv) = s {
        collect_rv_tys(rv, out);
    }
}

fn rename_stmt_tys(s: &mut Stmt, canon: &BTreeMap<String, String>) {
    if let Stmt::Assign(_, rv) = s {
        rename_rv_tys(rv, canon);
    }
}

fn collect_rv_tys(rv: &Rvalue, out: &mut BTreeSet<String>) {
    if let Rvalue::Cast { from, to, .. } = rv {
        collect_struct_names(from, out);
        collect_struct_names(to, out);
    }
    // An enum variant names its enum in the path prefix (`a::E::A` -> `a::E`);
    // that prefix must canonicalize with the enum's own type spelling.
    if let Rvalue::Enum { name, ops } = rv {
        if let Some(prefix) = name.rsplit_once("::").map(|(p, _)| p) {
            out.insert(prefix.to_string());
        }
        for op in ops {
            collect_op_tys(op, out);
        }
    }
    for op in rv_all_operands(rv) {
        collect_op_tys(op, out);
    }
}

/// The back-end identity of the type a struct aggregate-constant name refers
/// to. A std shape the back end models by its last segment (`std::ops::Range`
/// -> `Range`, matching `parse_ty`) keeps just that; a user aggregate keeps its
/// whole path (`a::A`), matching the identity `parse_ty` gave it and
/// `canonicalize_types` reconciled. Generic arguments are dropped either way.
pub(crate) fn struct_const_identity(name: &str) -> String {
    let base = strip_generics(name);
    let last = base.rsplit("::").next().unwrap_or(&base).trim();
    let std_shape = matches!(
        last,
        "Option" | "Range" | "Argument" | "ArgumentType" | "NonNull" | "PhantomData" | "Arguments"
    );
    if std_shape {
        last.to_string()
    } else {
        base
    }
}

/// The type a `Const::Agg` names: the whole path for a struct (`a::A`), the
/// enum prefix for a variant (`a::E::A` -> `a::E`). Returns the type name and
/// the `::variant` suffix to re-append, if the constant is a variant.
fn agg_type_and_variant(name: &str) -> (String, String) {
    let base = strip_generics(name);
    if let Some((prefix, variant)) = base.rsplit_once("::") {
        let last = prefix.rsplit("::").next().unwrap_or(prefix).trim();
        if last.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return (struct_const_identity(prefix), format!("::{variant}"));
        }
    }
    (struct_const_identity(&base), String::new())
}

fn collect_op_tys(op: &Operand, out: &mut BTreeSet<String>) {
    if let Operand::Const(Const::Agg { name, ops }) = op {
        out.insert(agg_type_and_variant(name).0);
        for o in ops {
            collect_op_tys(o, out);
        }
    }
}

fn rename_rv_tys(rv: &mut Rvalue, canon: &BTreeMap<String, String>) {
    if let Rvalue::Cast { from, to, .. } = rv {
        rename_struct_names(from, canon);
        rename_struct_names(to, canon);
    }
    if let Rvalue::Enum { name, ops } = rv {
        if let Some((prefix, variant)) = name.rsplit_once("::") {
            if let Some(c) = canon.get(prefix) {
                *name = format!("{c}::{variant}");
            }
        }
        for op in ops {
            rename_op_tys(op, canon);
        }
    }
    for op in rv_all_operands_mut(rv) {
        rename_op_tys(op, canon);
    }
}

fn rename_op_tys(op: &mut Operand, canon: &BTreeMap<String, String>) {
    if let Operand::Const(Const::Agg { name, ops }) = op {
        let (ty, suffix) = agg_type_and_variant(name);
        if let Some(c) = canon.get(&ty) {
            *name = format!("{c}{suffix}");
        }
        for o in ops {
            rename_op_tys(o, canon);
        }
    }
}

/// Every operand a statement's rvalue reads, for the type-canonicalization walk.
fn rv_all_operands(rv: &Rvalue) -> Vec<&Operand> {
    match rv {
        Rvalue::Use(op) => vec![op],
        Rvalue::Binary(_, a, b) => vec![a, b],
        Rvalue::Unary(_, a) => vec![a],
        Rvalue::Cast { op, .. } => vec![op],
        Rvalue::Repeat { op, .. } => vec![op],
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => ops.iter().collect(),
        Rvalue::WithOverflow { a, b, .. } => vec![a, b],
        _ => Vec::new(),
    }
}

fn rv_all_operands_mut(rv: &mut Rvalue) -> Vec<&mut Operand> {
    match rv {
        Rvalue::Use(op) => vec![op],
        Rvalue::Binary(_, a, b) => vec![a, b],
        Rvalue::Unary(_, a) => vec![a],
        Rvalue::Cast { op, .. } => vec![op],
        Rvalue::Repeat { op, .. } => vec![op],
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => ops.iter_mut().collect(),
        Rvalue::WithOverflow { a, b, .. } => vec![a, b],
        _ => Vec::new(),
    }
}

/// Replace each marked impl-method call target (`<IMPL_MARK>method`) with the
/// receiver-keyed symbol (`A__method`). The receiver type is read from the
/// call's first argument: its local's declared type, or, for an argument whose
/// own type is not yet resolved (a nested call result), the destination's type
/// where that is known. This runs after parsing so every `LocalDecl` is final.
fn resolve_impl_calls(prog: &mut Program) {
    for f in &mut prog.funcs {
        let locals: Vec<Ty> = f.locals.iter().map(|l| (*l.ty).clone()).collect();
        for b in &mut f.blocks {
            let (method, first_arg) = match &b.term {
                Terminator::Call { func: CallTarget::Direct(name), args, .. } => {
                    match name.strip_prefix(IMPL_MARK) {
                        Some(m) => (m.to_string(), args.first().cloned()),
                        None => continue,
                    }
                }
                _ => continue,
            };
            // An associated function (`A::make()`) has no receiver argument;
            // its definition is keyed by its bare name too, so fall back.
            let resolved = match first_arg.as_ref().and_then(|a| operand_receiver_ty(&locals, a)) {
                Some(recv) => impl_method_name(&recv, &method),
                None => method,
            };
            if let Terminator::Call { func, .. } = &mut b.term {
                *func = CallTarget::Direct(resolved);
            }
        }
    }
}

/// The receiver type of a method call's first argument: a reference/pointer
/// argument yields its pointee, an aggregate argument yields itself.
fn operand_receiver_ty(locals: &[Ty], op: &Operand) -> Option<String> {
    let place = match op {
        Operand::Copy(p) | Operand::Move(p) => p,
        _ => return None,
    };
    let ty = locals.get(place.local as usize)?;
    let inner = match ty {
        Ty::Ref(t) | Ty::Ptr(t) => &**t,
        other => other,
    };
    match inner {
        Ty::Struct(n) => Some(n.clone()),
        _ => None,
    }
}

fn strip_comment(line: &str) -> String {
    // A `//` inside a string literal is data, not a comment: a `const
    // "https://x"` or a template containing `//` must survive intact. Track
    // double-quoted runs (honouring backslash escapes) and cut only at a `//`
    // seen outside one.
    let b = line.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        match b[i] {
            b'\\' if in_str => {
                i += 2;
                continue;
            }
            b'"' => in_str = !in_str,
            b'/' if !in_str && b.get(i + 1) == Some(&b'/') => {
                return line[..i].to_string();
            }
            _ => {}
        }
        i += 1;
    }
    line.to_string()
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
    // A zero-sized alloc is printed on one line, `allocN (..) {}`.
    if header.ends_with("{}") {
        return Ok((id, data, start + 1));
    }
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

/// `const main::promoted[N]: &T = { ... }` — a promoted constant, reduced to
/// the bytes of the value it returns the address of. `None` means the shape is
/// not one the back end can fold.
fn parse_promoted(
    lines: &[String],
    start: usize,
    consts: &HashMap<String, i64>,
) -> Result<(String, Option<Promoted>, usize), String> {
    let header = lines[start].trim().trim_end_matches(';');
    let name = header
        .strip_prefix("const ")
        // The name/type separator is `: ` (colon-space); the `::` inside the
        // path has no space, so a plain `find(':')` would cut at the first `:`.
        .and_then(|r| r.find(": ").map(|c| r[..c].trim().to_string()))
        .ok_or_else(|| format!("mir: bad promoted header `{header}`"))?;
    // The promoted body opens with `= {` on the header line and closes on the
    // first column-0 `}`.
    let mut end = start + 1;
    while end < lines.len() {
        let raw = &lines[end];
        if !raw.is_empty() && raw.as_bytes()[0] != b' ' && raw.trim() == "}" {
            break;
        }
        end += 1;
    }
    if end >= lines.len() {
        return Err(format!("mir: unterminated promoted `{name}`"));
    }
    let body = &lines[start + 1..end];

    // Local declarations, as raw type text: `let mut _1: i32;`.
    let mut decls: HashMap<u32, String> = HashMap::new();
    for line in body {
        if let Some((n, ty_text)) = parse_let_raw(line.trim()) {
            decls.insert(n, ty_text);
        }
    }

    // The referent is the local the return takes the address of: `_0 = &_1;`.
    let mut referent: Option<u32> = None;
    for line in body {
        let t = line.trim();
        if let Some((lhs, rhs)) = split_assign(t) {
            let Some(dest) = parse_local_id(lhs.trim()) else { continue };
            if dest != 0 {
                continue;
            }
            if let Some(r) = rhs.trim().strip_prefix('&') {
                if let Some(x) = parse_local_id(r.trim()) {
                    referent = Some(x);
                }
            }
        }
    }
    let Some(ref_local) = referent else {
        return Ok((name, None, end + 1));
    };
    let Some(ty_text) = decls.get(&ref_local) else {
        return Ok((name, None, end + 1));
    };
    let size = referent_size(ty_text);

    // The value: `_1 = const V;`, `_1 = Add(const A, const B);`, or a zero-sized
    // construction `_1 = Q;`. Only a scalar referent is folded; anything else is
    // left unmodelled.
    let mut bytes: Vec<(i64, i64)> = Vec::new();
    let mut str_ref: Option<String> = None;
    for line in body {
        let t = line.trim();
        if let Some((lhs, rhs)) = split_assign(t) {
            if parse_local_id(lhs.trim()) == Some(ref_local) {
                let r = rhs.trim();
                if let Some(v) = parse_promoted_value(r, ty_text, consts) {
                    bytes.push((0, v));
                } else if ty_text.trim() == "&str" {
                    // A `&str` referent: the promoted blob holds the address of
                    // the string record the literal becomes.
                    if let Some(Const::Str(s)) = parse_const(r.strip_prefix("const ").unwrap_or(r)) {
                        str_ref = Some(s);
                    }
                }
            }
        }
    }
    if bytes.is_empty() && str_ref.is_none() {
        return Ok((name, None, end + 1));
    }
    Ok((name, Some(Promoted { size, bytes, str_ref }), end + 1))
}

/// The scalar a promoted referent is set to: a literal, a binary op folded over
/// two literals, a `const` item reference, or a `size_of`/zero-sized
/// construction (the type name used as a value).
fn parse_promoted_value(rhs: &str, ty_text: &str, consts: &HashMap<String, i64>) -> Option<i64> {
    let s = rhs.trim().trim_end_matches(';').trim();
    if let Some(v) = parse_const_value(s) {
        return Some(v);
    }
    // `_1 = const X;` — a reference to a `const` item, resolved to its value.
    if let Some(name) = s.strip_prefix("const ") {
        if let Some(v) = consts.get(name.trim()) {
            return Some(*v);
        }
    }
    // `_1 = std::mem::size_of::<i64>();` — a size query the front end left in
    // the promoted body rather than folding.
    if let Some(r) = s.strip_prefix("std::mem::size_of") {
        if let Some(t) = r.split('<').nth(1).and_then(|t| t.split('>').next()) {
            return Some(size_of_type_text(t.trim()));
        }
    }
    // `_1 = Q;` where `_1: Q` — a unit struct or unit variant, so a zero-sized
    // value with no bytes to store.
    if s == ty_text.trim() {
        return Some(0);
    }
    // `Add(const 3_i32, const 3_i32)` — a binary op over two literals, as the
    // MIR of a `macro_rules!` expansion like `$e + $e`.
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close <= open {
        return None;
    }
    let op = s[..open].trim();
    let (a, b) = split_top_comma(&s[open + 1..close])?;
    let x = parse_const_value(a.trim())?;
    let y = parse_const_value(b.trim())?;
    eval_binop(op, x, y, ty_text)
}

/// The byte size of a primitive type named in source text, for the `size_of`
/// shapes a promoted constant carries.
fn size_of_type_text(t: &str) -> i64 {
    match t {
        "i8" | "u8" | "bool" => 1,
        "i16" | "u16" => 2,
        "i32" | "u32" | "f32" | "char" => 4,
        _ => 8,
    }
}

/// Split `a, b` at the top-level comma (one not nested in brackets).
fn split_top_comma(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Fold a MIR binary op over two literals, at the referent's integer width.
fn eval_binop(op: &str, a: i64, b: i64, ty_text: &str) -> Option<i64> {
    let w = match ty_text.trim() {
        "i8" | "u8" => 8u32,
        "i16" | "u16" => 16,
        "i32" | "u32" => 32,
        _ => 64,
    };
    let mask: u64 = if w == 64 { u64::MAX } else { (1u64 << w) - 1 };
    let ua = (a as u64) & mask;
    let ub = (b as u64) & mask;
    let shift = ub & (w as u64 - 1);
    let r = match op {
        "Add" => ua.wrapping_add(ub),
        "Sub" => ua.wrapping_sub(ub),
        "Mul" => ua.wrapping_mul(ub),
        "BitXor" => ua ^ ub,
        "BitAnd" => ua & ub,
        "BitOr" => ua | ub,
        "Shl" => ua.wrapping_shl(shift as u32),
        "Shr" => {
            if ty_text.trim().starts_with('i') {
                ((a >> shift) as u64) & mask
            } else {
                ua >> shift
            }
        }
        // Division and remainder trap on a zero divisor, so a promoted body
        // that carries one is left unfolded rather than made to fault.
        "Div" if ub != 0 => {
            if ty_text.trim().starts_with('i') {
                (a.wrapping_div(b as i64)) as u64
            } else {
                ua / ub
            }
        }
        "Rem" if ub != 0 => {
            if ty_text.trim().starts_with('i') {
                (a.wrapping_rem(b as i64)) as u64
            } else {
                ua % ub
            }
        }
        _ => return None,
    };
    Some((r & mask) as i64)
}

/// `let mut _1: i32;` -> `(1, "i32")`, with the type kept as its source text so
/// `char` (four bytes) is not flattened to a word.
fn parse_let_raw(t: &str) -> Option<(u32, String)> {
    let t = t.strip_prefix("let ")?;
    let t = t.strip_prefix("mut ").unwrap_or(t);
    let t = t.strip_prefix('_')?;
    let colon = t.find(':')?;
    let n: u32 = t[..colon].trim().parse().ok()?;
    let ty = t[colon + 1..].trim().trim_end_matches(';').trim().to_string();
    Some((n, ty))
}

/// The referent size of a promoted constant, from its type text. Only the
/// scalar widths matter here; a `char` is four bytes and everything else the
/// back end folds is a word.
fn referent_size(ty_text: &str) -> i64 {
    if ty_text == "char" {
        4
    } else {
        8
    }
}

/// A scalar constant's value, for a promoted referent: an integer, a `bool`, or
/// a `char` (as its codepoint). Anything else is `None`.
fn parse_const_value(s: &str) -> Option<i64> {
    let s = s.strip_prefix("const ").unwrap_or(s).trim();
    match parse_const(s)? {
        Const::Int(n) => Some(n),
        Const::Bool(b) => Some(b as i64),
        Const::Char(c) => Some(c as i64),
        _ => None,
    }
}

fn parse_fn(lines: &[String], start: usize) -> Result<(Function, usize), String> {
    let header = lines[start].trim();
    let rest = header.strip_prefix("fn ").unwrap();
    let open = rest.find('(').ok_or_else(|| format!("mir: bad fn header `{header}`"))?;
    let close = matching(rest, open, '(', ')').ok_or_else(|| format!("mir: unbalanced fn args `{header}`"))?;
    let raw_name = rest[..open].trim().to_string();
    let arg_tys = parse_arg_tys(&rest[open + 1..close])?;
    // A method definition prints as `<impl at ...>::name` (or
    // `Type::<impl at ...>::name`), and rustc emits a distinct body per impl.
    // Key it by receiver type (`A__value`) so two impls of the same method name
    // cannot collide into one symbol; a call is resolved to the same key by
    // `resolve_impl_calls`. A free function keeps its bare name.
    let method = raw_name
        .contains("<impl")
        .then(|| raw_name.rsplit(">::").next().map(|m| m.trim().to_string()))
        .flatten();
    let recv = method_receiver_ty(&arg_tys);
    let mut name = match (&method, &recv) {
        (Some(m), Some(r)) if !m.is_empty() && !m.contains('<') => format!("{r}__{m}"),
        // An associated function has an impl-method name but no receiver
        // argument (`method_receiver_ty` yields `None`). Its *call* prints as
        // `Type::name` and `resolve_impl_calls` resolves it to the bare method
        // name, so the definition must be keyed that way too, or the call
        // targets a `\u{1}`-marked symbol that no definition carries.
        (Some(m), None) if !m.is_empty() && !m.contains('<') => m.clone(),
        _ => normalize_call_name(&raw_name).unwrap_or_else(|| raw_name.clone()),
    };
    // A `Drop` impl is renamed to `__rmc_drop_<T>`; `parse` records the mapping
    // so a `drop(_N)` terminator can find the right impl from `_N`'s type.
    if method.as_deref() == Some("drop") {
        if let Some(r) = &recv {
            name = format!("__rmc_drop_{r}");
        }
    }
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
    let ensure = |n: u32, locals: &mut Vec<LocalDecl>| {
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
        match parse_line_stmt(t, &locals) {
            Ok(Some(s)) => blocks[n as usize].stmts.push(s),
            Ok(None) => {}
            Err(e) => {
                // `parse_line_stmt` and its callees both prefix with `mir: `;
                // keep one. Then name the Rust feature when we recognise it.
                let e = e.strip_prefix("mir: ").unwrap_or(&e);
                let hint = match classify_unsupported(t) {
                    Some(h) => format!("\n  note: {h}"),
                    None => String::new(),
                };
                return Err(format!("mir: {e} (in `{name}`): {t}{hint}"));
            }
        }
    }

    // rustc always emits at least `bb0`, and the optimiser passes assume a
    // function has a first block (`facts_in[0]`, the entry). A hand-written or
    // corrupted MIR file can declare none, so reject it here rather than
    // indexing an empty vector later.
    if blocks.is_empty() {
        return Err(format!("mir: fn `{name}` has no basic blocks"));
    }
    // Every terminator must target a block that exists. rustc never emits a
    // dangling target, but a corrupted or hand-written `.mir` file can, and a
    // later pass indexes `blocks[target]` directly. Reject it at the boundary.
    let n_blocks = blocks.len() as u32;
    for (b, blk) in blocks.iter().enumerate() {
        for s in successors(&blk.term) {
            if s >= n_blocks {
                return Err(format!(
                    "mir: fn `{name}`: bb{b} targets bb{s}, which does not exist"
                ));
            }
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
fn parse_line_stmt(t: &str, locals: &[LocalDecl]) -> Result<Option<Stmt>, String> {
    if is_ignorable(t) || parse_call_terminator(t).is_some() {
        return Ok(None);
    }
    if let Some((lhs, rhs)) = split_assign(t) {
        let dest = parse_place(lhs)?;
        let rv = parse_rvalue(rhs)?;
        let rv = reclassify_enum_aggregate(rv, rhs, &dest, locals);
        return Ok(Some(Stmt::Assign(dest, rv)));
    }
    Err("unrecognised statement".to_string())
}

/// rustc prints a named-field enum variant (`e::Some { x }`) as a brace
/// aggregate, so by shape alone it is indistinguishable from a struct literal.
/// The destination's declared type disambiguates: when it is the enum itself,
/// the aggregate is the variant, and the back end must write the discriminant.
/// A struct spelled with a module path (`a::A { .. }`) has a path *prefix* (`a`)
/// that differs from its own type (`a::A`), so it stays an aggregate.
fn reclassify_enum_aggregate(rv: Rvalue, rhs: &str, dest: &Place, locals: &[LocalDecl]) -> Rvalue {
    let Rvalue::Aggregate(ops) = rv else { return rv };
    let Some(brace) = rhs.find('{') else { return Rvalue::Aggregate(ops) };
    let name = normalize_fmt_struct(rhs[..brace].trim());
    // A literal names an enum variant only when a `::variant` segment follows
    // the type path once its generics are stripped: `Option::<i64>::Some`
    // reduces to `Option::Some` (a variant), while `G::<T>` reduces to `G` (a
    // plain generic struct). Splitting the raw name would take `G::<T>`'s
    // turbofish as the variant and add a phantom tag to a struct.
    let base = strip_generics(name);
    let Some((prefix, variant)) = base.rsplit_once("::") else {
        return Rvalue::Aggregate(ops);
    };
    if prefix.is_empty() || variant.is_empty() {
        return Rvalue::Aggregate(ops);
    }
    if dest_enum_name(locals, dest).as_deref() != Some(prefix) {
        return Rvalue::Aggregate(ops);
    }
    Rvalue::Enum { name: name.to_string(), ops }
}

/// The enum name a place's type names, following at most a chain of derefs.
/// A field or index projection lands *inside* an aggregate, where a variant's
/// tag is not the whole value, so those yield `None` and the caller leaves the
/// aggregate alone.
fn dest_enum_name(locals: &[LocalDecl], p: &Place) -> Option<String> {
    let mut ty: &Ty = &locals.get(p.local as usize)?.ty;
    for proj in &p.proj {
        match proj {
            Proj::Deref => {
                ty = match ty {
                    Ty::Ref(t) | Ty::Ptr(t) => t,
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    match ty {
        Ty::Struct(n) => Some(n.clone()),
        _ => None,
    }
}

/// `_4 = f(args) -> [return: bbX, unwind ...]` is a call terminator.
fn parse_call_terminator(t: &str) -> Option<Terminator> {
    let (lhs, rhs) = split_assign(t)?;
    let (head, targets) = split_arrow(rhs)?;
    let (func, args) = parse_call_head(head).ok()?;
    let dest = parse_place(lhs).ok()?;
    let target = parse_success_target(targets);
    // `std::io::_print`/`_eprint` are the `print!`/`println!` sinks. Their last
    // path segment starts with `_`, so `normalize_call_name` would reject them
    // as indirect; map them to the runtime helpers that consume a
    // `core::fmt::Arguments` before that happens.
    let func = match func.trim() {
        "std::io::_print" | "core::fmt::_print" => CallTarget::Direct("__rmc_print".to_string()),
        "std::io::_eprint" | "core::fmt::_eprint" => CallTarget::Direct("__rmc_eprint".to_string()),
        "compare_bytes" => CallTarget::Direct("__rmc_compare_bytes".to_string()),
        _ if is_panic_fn(func.trim()) => CallTarget::Direct("__rmc_panic".to_string()),
        _ => match std_intrinsic_name(&func) {
            Some(name) => CallTarget::Direct(name),
            None => match normalize_call_name(&func) {
                Some(name) => CallTarget::Direct(name),
                None => CallTarget::Indirect,
            },
        },
    };
    Some(Terminator::Call { dest, func, args, target })
}

/// The `panic!`/`todo!`/`unreachable!`/`assert!` sinks. They print a message and
/// abort the process; the back end maps them to `__rmc_panic`, which writes a
/// fixed line to stderr and exits non-zero. The message text is a `&str` (or an
/// `Arguments`), so the helper ignores the argument rather than modelling the
/// `core::fmt` machinery these bring in.
fn is_panic_fn(name: &str) -> bool {
    matches!(
        name,
        "std::rt::panic_fmt"
            | "core::panicking::panic_fmt"
            | "core::panicking::panic"
            | "core::panicking::panic_str"
            | "core::panicking::panic_nounwind"
            | "core::panicking::panic_nounwind_fmt"
            | "std::panicking::begin_panic"
            | "core::panicking::panic_display"
    )
}

/// A std call the back end implements as a runtime helper, rewritten to that
/// helper's symbol so it survives `normalize_call_name` (which keeps only the
/// final path segment, `min`, which is not a symbol the runtime defines).
fn std_intrinsic_name(name: &str) -> Option<String> {
    let n = name.trim();
    // `<[u8]>::starts_with`/`ends_with` — the byte-slice predicates `str`'s
    // `starts_with`/`ends_with` inline to. rmc's `&[u8]` (and the `&[u8]` cast
    // of a string literal) is the address of a `{len: i64, bytes..}` record, so
    // the runtime helper reads that record, matching `__rmc_compare_bytes`.
    if n.contains("<impl [u8]>::") {
        if n.contains(">::starts_with") {
            return Some("__rmc_slice_starts_with".to_string());
        }
        if n.contains(">::ends_with") {
            return Some("__rmc_slice_ends_with".to_string());
        }
    }
    // `<&str as Pattern>::is_contained_in(pat, hay)` — the substring test
    // `str::contains` lowers to. The receiver (the pattern) is the first
    // argument, the haystack the second.
    if n.contains("as Pattern>::is_contained_in") {
        return Some("__rmc_str_contains".to_string());
    }
    // `<T as Ord>::min(a, b)` and `max`, for an integer `T`.
    let rest = n.strip_prefix('<')?;
    let (ty, tail) = rest.split_once(" as ")?;
    let (trait_name, method) = tail.split_once(">::")?;
    if trait_name == "Ord" && (method == "min" || method == "max") {
        let signed = ty.starts_with('i') || ty == "isize";
        let kind = if signed { "i" } else { "u" };
        return Some(format!("__rmc_{kind}{method}"));
    }
    None
}

/// The name a call should target, or `None` for a call that is not a plain
/// direct call (a function pointer, or an unresolved generic instantiation).
///
/// An inherent or trait method prints qualified (`Wide::total`,
/// `<core::ops::Range<i64> as Iterator>::next`). rmc's frontend drops the
/// qualifier, so the last path segment is the definition name; a leading
/// `::`-less segment after stripping an `<...>` qualifier is that name.
/// A sentinel prefixed to a call target that still needs its receiver type
/// resolved (`<impl at ...>::value`). The parser cannot key it without the
/// argument types, so `parse` leaves the marker and `resolve_impl_calls` fills
/// it in once the whole program (and thus every struct layout) is known.
const IMPL_MARK: char = '\u{1}';

fn normalize_call_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('_') {
        return None;
    }
    if !name.contains("::") && !name.contains('<') {
        return Some(name.to_string());
    }
    // An impl method (`<impl at ...>::value`, `Type::<impl at ...>::value`) is
    // returned with a marker and its method name; `resolve_impl_calls` later
    // replaces the marker with the receiver type derived from the arguments.
    if name.contains("<impl") {
        let last = name.rsplit(">::").next()?.trim();
        if last.is_empty() || last.starts_with('_') || last.contains('<') {
            return None;
        }
        return Some(format!("{IMPL_MARK}{last}"));
    }
    // An inherent method call prints as `Type::method` (a capitalised first
    // segment, a lowercase last). It carries the same marker as an impl-method
    // definition so the two resolve to one receiver-keyed symbol. A
    // module-qualified free function (`m::helper`) has a lowercase first
    // segment and is left alone.
    let head = name.split('<').next().unwrap_or(name);
    let segs: Vec<&str> = head.split("::").map(str::trim).collect();
    let first = segs.first().copied().unwrap_or(head);
    let tail = segs.last().copied().unwrap_or(head);
    let is_ident = !tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let is_method = tail.chars().next().is_some_and(|c| c.is_ascii_lowercase());
    // `Type::method` (a capitalised first segment, a lowercase last).
    if first != tail
        && first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && is_method
        && is_ident
    {
        return Some(format!("{IMPL_MARK}{tail}"));
    }
    // `module::Type::method` — the printer qualifies the receiver with the
    // modules it is named from (`a::A::value`), so a capitalised
    // second-to-last segment is the receiver and the call is a method too.
    // A `module::function` path has a lowercase second-to-last segment and is
    // left as a free function.
    if segs.len() >= 2 {
        let penult = segs[segs.len() - 2];
        if penult.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && is_method
            && is_ident
        {
            return Some(format!("{IMPL_MARK}{tail}"));
        }
    }
    // Drop a leading `<...>` qualifier, then take the final path segment.
    let mut rest = name;
    if rest.starts_with('<') {
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
            // `assume(cond)` is an optimisation hint: rustc already relied on it
            // to justify the surrounding code, and codegen needs no action.
            | "assume"
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
        // `switchInt(OverflowChecks)` branches on whether the crate was built
        // with overflow checks. The front end always runs `-O` with them off,
        // so the `0:` arm is the one taken; the `otherwise` arm is dead. Folding
        // it here keeps the branch out of the back end rather than teaching it
        // about a compile-time constant it can never see vary.
        if let Some(target) = parse_overflow_checks_switch(t) {
            return Some(target);
        }
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
        // `drop(_N) -> [return: bbX, ...]`. The place names the value whose
        // destructor runs; the back end resolves `_N`'s type to its `Drop::drop`
        // impl. A `drop` with no target is `unreachable` (the value is dropped
        // and control does not continue), which the caller cannot represent, so
        // reject it rather than miscompile.
        let open = t.find('(')?;
        let close = matching(t, open, '(', ')')?;
        let place = parse_place(t[open + 1..close].trim()).ok()?;
        let (_, targets) = split_arrow(t)?;
        let target = parse_success_target(targets)?;
        let _ = rest;
        return Some(Terminator::Drop { place, target });
    }
    if t.starts_with("StorageLive") || t.starts_with("StorageDead") || t == "Nop;" {
        return None;
    }
    None
}

/// `switchInt(OverflowChecks) -> [0: bbX, otherwise: bbY]` folds to `goto bbX`:
/// the front end always compiles with overflow checks off, so `0` is the only
/// arm that can be taken.
fn parse_overflow_checks_switch(t: &str) -> Option<Terminator> {
    let open = t.find('(')?;
    let close = matching(t, open, '(', ')')?;
    if t[open + 1..close].trim() != "OverflowChecks" {
        return None;
    }
    let after = t[close + 1..].trim().strip_prefix("->")?.trim();
    let inner = after.trim_start_matches('[').trim_end_matches(']').trim_end_matches(';');
    for part in inner.split(',') {
        let p = part.trim();
        if let Some((val, bb)) = p.split_once(':') {
            if val.trim().trim_end_matches("_bool").trim() == "0" {
                return Some(Terminator::Goto(parse_bb_id(bb.trim())?));
            }
        }
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
        } else if let Some(r2) = r.strip_prefix("as ") {
            // `as Variant` — the payload of an enum variant, e.g. `(_1 as B).0`.
            // The variant name runs up to the `)` that closes the enclosing
            // `(PLACE as Variant)`.
            let end = r2.find(')').unwrap_or(r2.len());
            proj.push(Proj::Variant(r2[..end].trim().to_string()));
            rest = &r2[end..];
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
    // A reified formatter, `format_args!`'s lowering of a placeholder. It may
    // stand alone (`<i64 as fmt::Display>::fmt`) or be cast to a function
    // pointer (`... as unsafe fn(NonNull<()>, &mut Formatter) -> Result
    // (Transmute)`); either way the value is the formatter, so keep it as a
    // `Fmt` operand. This must run before the generic cast split, which would
    // otherwise split at the ` as ` inside the `<i64 as ...>` path.
    if let Some(pos) = s.find(">::fmt") {
        let end = pos + ">::fmt".len();
        if let Some((ty_name, trait_name)) = parse_reify_fn(&s[..end]) {
            let rest = s[end..].trim();
            if let Some(ty_kind) = rest.strip_prefix("as ") {
                let (ty, kind) = parse_cast_target(ty_kind)?;
                return Ok(Rvalue::Cast {
                    from: Box::new(Ty::Other),
                    to: Box::new(ty),
                    op: Operand::Fmt { ty_name, trait_name },
                    kind,
                });
            }
            return Ok(Rvalue::Use(Operand::Fmt { ty_name, trait_name }));
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
    // A tuple literal `(a, b, ..)` — consecutive words, like an array.
    if s.starts_with('(') && s.ends_with(')') && matching(s, 0, '(', ')') == Some(s.len() - 1) {
        let mut ops = Vec::new();
        for p in split_top_args(&s[1..s.len() - 1]) {
            if p.trim().is_empty() {
                continue;
            }
            ops.push(parse_operand(p.trim())?);
        }
        return Ok(Rvalue::Aggregate(ops));
    }
    // A struct/tuple literal: `Name { f: v, .. }` or `Name(v, ..)`.
    if let Some(brace) = s.find('{') {
        let name = s[..brace].trim();
        if !name.is_empty() && !name.contains("const ") && !name.contains('(') {
            let inner = braced_inner(s, brace).ok_or_else(|| format!("mir: unbalanced aggregate `{s}`"))?;
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
            // An enum variant carries its name so the backend can find the
            // variant's payload offset; a plain struct is an `Aggregate`.
            // The `format_args!` lowering spells its structs with paths that
            // contain `::` (`core::fmt::rt::Argument::<'_> { .. }`,
            // `...ArgumentType::<'_>::Placeholder { .. }`), so those are
            // normalised to a bare name instead of being mistaken for variants.
            let name = normalize_fmt_struct(name);
            if name.contains("::") && is_enum_variant_name(name) {
                return Ok(Rvalue::Enum { name: name.to_string(), ops });
            }
            return Ok(Rvalue::Aggregate(ops));
        }
    }
    // `Enum::Variant(a, b)` — a tuple-variant constructor.
    if let Some(open) = s.find('(') {
        let name = s[..open].trim();
        if name.contains("::") && s.ends_with(')') && matching(s, open, '(', ')') == Some(s.len() - 1) {
            let args = split_top_args(&s[open + 1..s.len() - 1]);
            let mut ops = Vec::new();
            for a in args {
                if a.trim().is_empty() {
                    continue;
                }
                ops.push(parse_operand(a.trim())?);
            }
            return Ok(Rvalue::Enum { name: name.to_string(), ops });
        }
    }
    // An enum variant constructor `Enum::Variant { .. }` was handled above; a
    // unit variant `Enum::Variant` is a bare name.
    // `Len(PLACE)`
    if let Some(r) = s.strip_prefix("Len(") {
        // Strip exactly the closing paren, not every trailing one: a place such
        // as `(*_1)` ends in a paren of its own, and `trim_end_matches` would
        // eat both, leaving the deref unbalanced.
        let inner = r.strip_suffix(')').unwrap_or(r);
        return Ok(Rvalue::Len(parse_place(inner.trim())?));
    }
    // `discriminant(PLACE)`
    if let Some(r) = s.strip_prefix("discriminant(") {
        let inner = r.strip_suffix(')').unwrap_or(r);
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
    // A fieldless enum variant used as a value (`b::E::B`). rustc prints a
    // variant *with* a payload as a constructor call, but a unit variant is a
    // bare path, and the back end needs the enum and variant to write the tag.
    // Both the last segment and the one before it must be type-cased, so an
    // associated constant (`i64::MAX`, `std::u64::MAX`) is not mistaken for a
    // variant.
    if s.contains("::") && !s.contains(char::is_whitespace) {
        let segs: Vec<&str> = s.split("::").collect();
        let camel = |x: &str| {
            x.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && x.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        let n = segs.len();
        if n >= 2 && camel(segs[n - 1]) && camel(segs[n - 2]) {
            return Ok(Rvalue::Enum { name: s.to_string(), ops: Vec::new() });
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
        "Transmute" => CastKind::Transmute,
        _ => CastKind::Other,
    }
}

fn parse_operand(s: &str) -> Result<Operand, String> {
    let s = s.trim();
    // `<T as core::fmt::Display>::fmt` — a reified formatter. `format_args!`
    // lowers every `{}`/`{:?}`/... placeholder to one of these, so the back end
    // must recognise it rather than treat it as a function pointer.
    if let Some((ty_name, trait_name)) = parse_reify_fn(s) {
        return Ok(Operand::Fmt { ty_name, trait_name });
    }
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
    // A bare constant, e.g. `()`, a `"str"` literal, or a named constant the
    // printer wrote without the `const` keyword.
    if let Some(c) = parse_const(s) {
        return Ok(Operand::Const(c));
    }
    Err(format!("mir: not an operand `{s}`"))
}

/// Map a raw MIR fragment the parser rejected to the user-visible Rust feature
/// it comes from, so the diagnostic names the limitation instead of quoting a
/// rustc-internal type. This is a message-quality layer only: an unrecognised
/// fragment still fails, it just fails with advice.
pub fn classify_unsupported(fragment: &str) -> Option<&'static str> {
    let s = fragment;
    let has = |needle: &str| s.contains(needle);
    // Order matters: the most specific construct first.
    if has("::fmt::Display>::fmt")
        || has("::fmt::Debug>::fmt")
        || has("::fmt::LowerHex>::fmt")
        || has("::fmt::UpperHex>::fmt")
        || has("::fmt::Binary>::fmt")
        || has("::fmt::Pointer>::fmt")
        || has("::fmt::Octal>::fmt")
    {
        return Some("`{}`/`{:?}` formatting (rustc's `core::fmt`) is not implemented on the MIR path; write the value with `print_i64`/`print_str` instead");
    }
    if has("PointerCoercion(ReifyFnPointer") {
        return Some("taking a function or method as a value is not implemented on the MIR path");
    }
    if has("ReifyFnPointer") || has("as *const ()") && has("unsafe fn") {
        return Some("function pointers are not implemented on the MIR path");
    }
    if has("as &dyn ") || has("Unsize") && has("dyn") {
        return Some("`dyn Trait` objects are not implemented on the MIR path");
    }
    if has("PtrMetadata") {
        return Some("`PtrMetadata` on a value that is not a `&str`/`&[T]` literal is not implemented on the MIR path");
    }
    if has("NonZero") || has("UsizeNoHighBit") || has("Unique::") || has("alloc::raw_vec") {
        return Some("the standard library's container internals (`Vec`/`String`) are not implemented on the MIR path");
    }
    if has("Vec<") || has("String") || has("HashMap") || has("Box<") || has("Rc<") {
        return Some("the standard library's heap containers (`Vec`/`String`/`Box`/`Rc`) are not implemented on the MIR path");
    }
    if has("Option<") && has("as core::fmt") {
        return Some("`{:?}` on an `Option` is not implemented on the MIR path");
    }
    if has("::from_iter") || has("SpecFromIter") {
        return Some("iterator adapters (`map`/`filter`/`collect`) are not implemented on the MIR path");
    }
    if has("as Fn<") || has("FnOnce") || has("FnMut") {
        return Some("closures and the `Fn` traits are not implemented on the MIR path");
    }
    if has("{closure@") || has("closure#") {
        return Some("closures are not implemented on the MIR path");
    }
    if has("f64") || has("f32") {
        return Some("floating-point types are not implemented on the MIR path");
    }
    if has("i128") || has("u128") {
        return Some("128-bit integers are not implemented on the MIR path");
    }
    if has("::promoted[") {
        return Some("a promoted constant of a shape the back end cannot fold (an aggregate, or a reference to one) is not implemented on the MIR path");
    }
    None
}

fn parse_const(s: &str) -> Option<Const> {
    let s = s.trim();
    // An aggregate constant is printed with its own `const` prefix, e.g. the
    // elements of `[const P {{ .. }}, ..]`; strip it and parse the value.
    if let Some(r) = s.strip_prefix("const ") {
        return parse_const(r.trim());
    }
    if s == "()" {
        return Some(Const::Unit);
    }
    if s == "true" {
        return Some(Const::Bool(true));
    }
    if s == "false" {
        return Some(Const::Bool(false));
    }
    // A `char` literal (`'A'`, `'\n'`, `'\u{1F600}'`). Rust's `char` is a
    // 32-bit Unicode scalar, so the value is the codepoint.
    if let Some(c) = parse_char_literal(s) {
        return Some(Const::Char(c));
    }
    // A zero-sized constant: `PhantomData::<..>` and the like. It takes no
    // storage, so the aggregate that lists it must not reserve a word.
    if s.starts_with("PhantomData") {
        return Some(Const::Zst);
    }
    if let Some(bytes) = parse_byte_string(s) {
        return Some(Const::Bytes(bytes));
    }
    if s.starts_with('"') {
        let inner = s.trim_start_matches('"').trim_end_matches('"');
        return Some(Const::Str(unescape(inner)));
    }
    if let Some(n) = parse_named_const(s) {
        return Some(Const::Int(n));
    }
    // An `f64`/`f32` literal: `5.5f64`, `2f64`, `-0f64`, `1.0E+100f64`, and
    // the special forms `+Inf_f64` / `-Inf_f64` / `NaN_f64` the printer uses
    // for the non-finite values. The suffix is the width; a value with no
    // suffix (`const 5.5`) does not appear, so requiring one keeps this from
    // swallowing an integer.
    if let Some(f) = parse_float_literal(s) {
        return Some(f);
    }
    // `main::promoted[N]` — a reference to a promoted constant. The parser folds
    // the promoted body into `Program::promoted`; the back end materialises its
    // `.rodata` blob. A name that was not folded is rejected at codegen.
    if s.contains("::promoted[") {
        return Some(Const::Named(s.to_string()));
    }
    // `{allocN: TY}` — a static allocation, possibly with an interior
    // offset printed as `{allocN: TY[place]}`.
    if s.starts_with('{') {
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
        // `{ x: 1_i64, y: 2_i64 }` — an unnamed aggregate, or a named one the
        // printer wrote as `Name { .. }` (handled before the brace fallback).
        if let Some(inner) = braced_inner(s, 0) {
            let mut ops = Vec::new();
            for part in split_top_args(inner) {
                let p = part.trim();
                if p.is_empty() {
                    continue;
                }
                let v = p.split_once(':').map(|(_, v)| v).unwrap_or(p);
                ops.push(parse_operand(v.trim()).ok()?);
            }
            return Some(Const::Agg { name: String::new(), ops });
        }
        return Some(Const::Named(s.to_string()));
    }
    // `Name { f: v, .. }` — a struct or enum-variant constant.
    if let Some(brace) = s.find('{') {
        let name = s[..brace].trim();
        if !name.is_empty() && !name.contains(' ') && !name.contains('(') {
            if let Some(inner) = braced_inner(s, brace) {
                let mut ops = Vec::new();
                for part in split_top_args(inner) {
                    let p = part.trim();
                    if p.is_empty() {
                        continue;
                    }
                    let v = p.split_once(':').map(|(_, v)| v).unwrap_or(p);
                    ops.push(parse_operand(v.trim()).ok()?);
                }
                return Some(Const::Agg { name: name.to_string(), ops });
            }
        }
    }
    // `E::A` / `E::Variant(args)` — an enum-variant constant. The qualifier may
    // carry generic arguments (`Option::<i32>::Some(1_i32)`), which are dropped
    // so the name matches the one `split_ev` produces at codegen.
    if let Some(open) = s.find('(') {
        if s.ends_with(')') && !s.starts_with('{') {
            let name = strip_generics(s[..open].trim());
            if !name.is_empty() && name.contains("::") {
                let args = split_top_args(&s[open + 1..s.len() - 1]);
                let mut ops = Vec::new();
                for a in args {
                    if a.trim().is_empty() {
                        continue;
                    }
                    ops.push(parse_operand(a.trim()).ok()?);
                }
                return Some(Const::Agg { name, ops });
            }
        }
    }
    // `(a, b, ..)` — a tuple constant, laid out as consecutive words.
    if s.starts_with('(') && s.ends_with(')') {
        let inner = &s[1..s.len() - 1];
        let mut ops = Vec::new();
        for part in split_top_args(inner) {
            let p = part.trim();
            if p.is_empty() {
                continue;
            }
            ops.push(parse_operand(p).ok()?);
        }
        return Some(Const::Agg { name: String::new(), ops });
    }
    if s.contains("::") {
        let bare = strip_generics(s);
        if bare.split("::").all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            return Some(Const::Agg { name: bare, ops: Vec::new() });
        }
    }
    if let Some(n) = parse_int_literal(s) {
        return Some(Const::Int(n));
    }
    None
}

/// Drop `<...>` generic-argument groups from a path, so `Option::<i32>::None`
/// and `Option::None` are the same name. Nesting is handled by depth counting.
fn strip_generics(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '<' => {
                depth += 1;
                // `Option::<i32>::None` — the `::` introducing the group goes
                // with it, or the name keeps a stray `::`.
                if out.ends_with("::") {
                    out.truncate(out.len() - 2);
                }
            }
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// `i64::MIN`, `u64::MAX`, `usize::MAX`, `i32::MIN`, ... — the associated
/// integer constants MIR prints by name.
fn parse_named_const(s: &str) -> Option<i64> {
    // `i64::MIN`, and the qualified form rustc prints inside a promoted body:
    // `core::num::<impl i64>::MIN`. The `impl` marker names the type between
    // `<` and `>`; strip everything up to and including it so both shapes
    // reduce to `TY::NAME`.
    let s = match s.find("::<impl ") {
        Some(at) => {
            let rest = &s[at + "::<impl ".len()..];
            let ty = rest.split('>').next()?.trim();
            // Rebuild as `TY::NAME`, where NAME is the last path segment.
            let name = s.rsplit("::").next()?.trim();
            return parse_int_assoc(ty, name);
        }
        None => s,
    };
    let (ty, name) = s.split_once("::")?;
    parse_int_assoc(ty.trim(), name.trim())
}

/// `i64::MIN`, `u8::MAX`, ... — the associated integer constant of a named type.
fn parse_int_assoc(ty: &str, name: &str) -> Option<i64> {
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
        // An unsigned max is its own bit pattern, not all ones: `u8::MAX` is
        // 255. Only `u64`/`usize` fill the 64-bit word (and overflow the shift).
        "MAX" if bits == 64 => Some(-1),
        "MAX" => Some((1i64 << bits) - 1),
        _ => None,
    }
}

/// An `f64`/`f32` literal, as the MIR printer writes it. The suffix carries the
/// width: `5.5f64` for a finite value, and `+Inf_f64` / `-Inf_f64` / `NaN_f64`
/// (an underscore before the width) for the non-finite ones. The decimal text
/// is parsed by Rust's own `f64::from_str`, which is correctly rounded to
/// nearest-even, so the value matches rustc's exactly.
fn parse_float_literal(s: &str) -> Option<Const> {
    let s = s.trim();
    for (suffix, is_f32) in [("_f64", false), ("_f32", true)] {
        if let Some(body) = s.strip_suffix(suffix) {
            let v = match body {
                "+Inf" => f64::INFINITY,
                "-Inf" => f64::NEG_INFINITY,
                "NaN" => f64::NAN,
                _ => return None,
            };
            return Some(Const::Float(v, is_f32));
        }
    }
    let (body, is_f32) = if let Some(b) = s.strip_suffix("f64") {
        (b, false)
    } else if let Some(b) = s.strip_suffix("f32") {
        (b, true)
    } else {
        return None;
    };
    // A bare `f64`/`f32` (a type fragment) leaves an empty body, which fails to
    // parse; an integer suffix such as `2_i64` never reaches here.
    let v = if is_f32 {
        body.parse::<f32>().ok()? as f64
    } else {
        body.parse::<f64>().ok()?
    };
    Some(Const::Float(v, is_f32))
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
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            // Copy a whole UTF-8 sequence so a multi-byte literal stays intact.
            let len = utf8_len(b[i]);
            let end = (i + len).min(b.len());
            out.push_str(&s[i..end]);
            i = end;
            continue;
        }
        i += 1;
        let Some(&c) = b.get(i) else {
            out.push('\\');
            break;
        };
        i += 1;
        match c {
            b'n' => out.push('\n'),
            b't' => out.push('\t'),
            b'r' => out.push('\r'),
            b'0' => out.push('\0'),
            b'\\' => out.push('\\'),
            b'"' => out.push('"'),
            b'\'' => out.push('\''),
            b'x' => {
                // `\xNN` -- exactly two hex digits.
                if let (Some(hi), Some(lo)) = (
                    b.get(i).and_then(|c| (*c as char).to_digit(16)),
                    b.get(i + 1).and_then(|c| (*c as char).to_digit(16)),
                ) {
                    out.push((hi * 16 + lo) as u8 as char);
                    i += 2;
                } else {
                    out.push_str("\\x");
                }
            }
            b'u' => {
                // `\u{...}` -- braced hex, e.g. `\u{1F600}`.
                let rest = &s[i..];
                if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
                    if let Ok(cp) = u32::from_str_radix(inner.0, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                        i += inner.0.len() + 2;
                    }
                } else {
                    out.push_str("\\u");
                }
            }
            other => {
                out.push('\\');
                out.push(other as char);
            }
        }
    }
    out
}

/// The byte length of the UTF-8 sequence whose lead byte is `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

/// `'A'`, `'\n'`, `'\u{1F600}'` — a `char` literal as MIR prints it, returned
/// as its Unicode scalar value. `char` is 32 bits, so the value is the
/// codepoint, not a byte.
fn parse_char_literal(s: &str) -> Option<u32> {
    let body = s.strip_prefix('\'')?.strip_suffix('\'')?;
    let b = body.as_bytes();
    if b.is_empty() {
        return None;
    }
    if b[0] != b'\\' {
        // A single literal character (which may be multi-byte UTF-8).
        let mut chars = body.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        return Some(c as u32);
    }
    match *b.get(1)? {
        b'n' => Some('\n' as u32),
        b't' => Some('\t' as u32),
        b'r' => Some('\r' as u32),
        b'0' => Some(0),
        b'\\' => Some('\\' as u32),
        b'\'' => Some('\'' as u32),
        b'"' => Some('"' as u32),
        b'x' => {
            let hi = (*b.get(2)? as char).to_digit(16)?;
            let lo = (*b.get(3)? as char).to_digit(16)?;
            Some(hi * 16 + lo)
        }
        b'u' => {
            // `\u{1F600}` — the braces hold hex digits.
            let inner = body.strip_prefix("\\u{")?.strip_suffix('}')?;
            u32::from_str_radix(inner, 16).ok()
        }
        _ => None,
    }
}

/// `b"..."` — a byte-string literal, as `rustc --emit=mir` prints it. The
/// escapes are the Rust byte-string escapes (`\xNN`, `\n`, `\t`, `\\`, `\"`,
/// `\0`, `\r`); the content is raw bytes, not a `str`.
fn parse_byte_string(s: &str) -> Option<Vec<u8>> {
    let body = s.strip_prefix("b\"")?.strip_suffix('"')?;
    let mut out = Vec::new();
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 1;
        let c = *b.get(i)?;
        i += 1;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'0' => out.push(0),
            b'\\' => out.push(b'\\'),
            b'"' => out.push(b'"'),
            b'\'' => out.push(b'\''),
            b'x' => {
                let hi = (*b.get(i)? as char).to_digit(16)?;
                let lo = (*b.get(i + 1)? as char).to_digit(16)?;
                out.push((hi * 16 + lo) as u8);
                i += 2;
            }
            _ => out.push(c),
        }
    }
    Some(out)
}

/// `<T as core::fmt::Trait>::fmt` — a reified formatter, as `format_args!`
/// lowers it. Returns the formatted type and the trait name (`Display`,
/// `Debug`, `LowerHex`, ...). The MIR path maps this to one of its own
/// runtime formatters, so the exact trait only selects the numeric base.
fn parse_reify_fn(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    let rest = s.strip_prefix('<')?;
    let as_pos = rest.find(" as ")?;
    let ty = rest[..as_pos].trim();
    let after = &rest[as_pos + 4..];
    let trait_path = after.strip_suffix(">::fmt")?;
    if !trait_path.contains("fmt") {
        return None;
    }
    let trait_name = trait_path.rsplit("::").next().unwrap_or(trait_path).trim();
    Some((ty.to_string(), trait_name.to_string()))
}

/// The `format_args!` lowering names its own structs with full paths
/// (`core::fmt::rt::Argument::<'_>`, `core::fmt::rt::ArgumentType::<'_>`,
/// `Arguments::<'_>`). Reduce those to a bare name so the backend's aggregate
/// handling sees the shape rather than an `Enum::Variant`. Any other path is
/// left alone, so a genuine `Enum::Variant` is still treated as one.
fn normalize_fmt_struct(name: &str) -> &str {
    let head = name.split("::<").next().unwrap_or(name);
    let last = head.rsplit("::").next().unwrap_or(head).trim();
    if last == "Argument" || last == "ArgumentType" || last == "Arguments" {
        return last;
    }
    name
}

/// Whether a struct-literal path names an enum variant rather than a struct.
///
/// rustc prints both struct and enum literals with a full path. A generic
/// *struct* ends with its type's generics (`std::slice::Iter::<'_, i32>`),
/// while an enum *variant* has a path segment after the type's generics
/// (`core::option::Option::<i64>::Some`) or no generics at all
/// (`core::option::Option::Some`). Treating a generic struct as a variant
/// adds a phantom tag word and shifts every field by one, which silently
/// corrupts the value.
fn is_enum_variant_name(name: &str) -> bool {
    match name.rfind("::<") {
        None => {
            // No generics. `Enum::Variant` has a type-cased segment before the
            // last (`core::option::Option::Some`, `b::E::B`), while a struct the
            // printer qualified by its module has a lowercase one there
            // (`a::A`, `outer::inner::C`). Treating the latter as a variant adds
            // a phantom tag word and shifts every field by one.
            let mut segs = name.rsplit("::");
            let _last = segs.next();
            match segs.next() {
                Some(penult) => penult
                    .trim()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase()),
                // The caller only reaches here when the name contains `::`.
                None => true,
            }
        }
        Some(i) => name[i + 3..].contains("::"),
    }
}

/// The receiver type of a method's `_1` argument (`_1: &mut Guard` ->
/// `Guard`), which keys the method's qualified symbol. A non-aggregate receiver
/// (`&i64`) has no impl type to qualify by.
fn method_receiver_ty(arg_tys: &[(u32, Ty)]) -> Option<String> {
    let (_, ty) = arg_tys.iter().find(|(i, _)| *i == 1)?;
    let inner = match ty {
        Ty::Ref(t) | Ty::Ptr(t) => &**t,
        other => other,
    };
    match inner {
        Ty::Struct(n) => Some(n.clone()),
        _ => None,
    }
}

/// The emitted symbol for an impl method: `<Receiver>__<method>`, so two impls
/// of the same method name on different types stay distinct.
fn impl_method_name(recv: &str, method: &str) -> String {
    format!("{recv}__{method}")
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
        "u8" => Ok(Ty::U8),
        "i8" => Ok(Ty::signed_int(1)),
        "f64" => Ok(Ty::F64),
        "f32" => Ok(Ty::F32),
        // `usize`/`isize` are 64-bit here (the target is x86-64). Each width
        // keeps its signedness so the exact op and the casts can honour it.
        "u64" | "usize" => Ok(Ty::unsigned_int(8)),
        "isize" => Ok(Ty::I64),
        "u32" => Ok(Ty::unsigned_int(4)),
        "i32" => Ok(Ty::signed_int(4)),
        "u16" => Ok(Ty::unsigned_int(2)),
        "i16" => Ok(Ty::signed_int(2)),
        "u128" | "i128" => Ok(Ty::Int { width: 16, signed: !s.starts_with('u') }),
        "char" => Ok(Ty::unsigned_int(4)),
        "()" => Ok(Ty::Unit),
        "!" => Ok(Ty::Unit),
        _ => {
            if s.contains("fn(") || s.contains("dyn ") {
                return Ok(Ty::Other);
            }
            // `core::fmt::Arguments` is the `format_args!` payload: two words,
            // a template pointer and an argument-array pointer. It is passed by
            // address like any other two-field struct, so the backend must know
            // its shape rather than treat it as one scalar word.
            if s.contains("fmt::Arguments") {
                return Ok(Ty::Struct("Arguments".to_string()));
            }
            // `core::option::Option<i64>` and `core::ops::Range<i64>` are
            // aggregates the backend models by their last path segment.
            let base = s.split('<').next().unwrap_or(s).trim();
            let last = base.rsplit("::").next().unwrap_or(base).trim();
            if last == "Option" || last == "Range" {
                return Ok(Ty::Struct(last.to_string()));
            }
            // The `format_args!` lowering builds a tree of these types. The
            // backend models each by its last path segment so the aggregate
            // layout (`Argument` = one 16-byte `ArgumentType`, `ArgumentType`
            // = one word) matches what rustc's MIR stores into.
            if last == "Argument"
                || last == "ArgumentType"
                || last == "NonNull"
                || last == "PhantomData"
            {
                return Ok(Ty::Struct(last.to_string()));
            }
            if s.contains("::") {
                // A qualified user-defined aggregate (`m::Pair`, `crate::E`)
                // reaches here after the std shapes above. Keep its *whole*
                // path as its identity, not just the last segment: two distinct
                // types can share a last segment (`a::A` and `b::A`), and
                // folding both to `A` makes their layouts and method symbols
                // collide. `canonicalize_types` later shortens a path back to
                // its last segment when that segment is unambiguous, so a type
                // printed bare in its own module still matches.
                if let Some(path) = user_aggregate_path(s) {
                    return Ok(Ty::Struct(path));
                }
                Ok(Ty::Other)
            } else {
                Ok(Ty::Struct(s.to_string()))
            }
        }
    }
}

// --- small string utilities ------------------------------------------------

/// The path-qualified *user aggregate* type name, if `s` names one
/// (`m::Pair` -> `m::Pair`, `crate::E` -> `crate::E`). The whole path is kept so
/// two same-named types in different modules stay distinct; `canonicalize_types`
/// shortens it later where the last segment is unambiguous.
///
/// `None` for a function-pointer or trait-object type, whose last segment is
/// not a type, so `parse_ty` keeps treating those as scalars. Only types the
/// earlier std-shape arms did not already claim reach here, so
/// `std::fmt::Arguments` and the `format_args!` helpers are handled before this
/// is consulted.
fn user_aggregate_path(s: &str) -> Option<String> {
    // Drop a leading `<...>` qualifier (`<T as Trait>::Assoc`).
    let mut rest = s.trim();
    if rest.starts_with('<') {
        let close = matching(rest, 0, '<', '>')?;
        rest = rest[close + 1..].trim_start_matches("::").trim();
    }
    // A type argument list may follow the last segment; cut at the first `<`.
    let head = rest.split('<').next().unwrap_or(rest);
    let last = head.rsplit("::").next().unwrap_or(head).trim();
    let ok = !last.is_empty()
        && last.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then(|| head.trim().to_string())
}

/// Index of the matching close bracket for the opener at `open`.
fn matching(s: &str, open: usize, oc: char, cc: char) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = string_end(bytes, i) {
            i = end;
            continue;
        }
        if i >= open {
            let c = bytes[i] as char;
            if c == oc {
                depth += 1;
            } else if c == cc {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

/// If `bytes[i]` starts a string literal (`"..."` or `b"..."`), return the
/// index just past its closing quote; otherwise `None`. Escapes are skipped so
/// an escaped quote does not end the literal early. The MIR scanners must not
/// treat a `)`/`,`/`;`/`as` inside a literal (e.g. a `format_args!` template or
/// a `b"\x03a)b..."` byte string) as syntax.
fn string_end(bytes: &[u8], i: usize) -> Option<usize> {
    let start = if bytes[i] == b'"' {
        i
    } else if bytes[i] == b'b' && bytes.get(i + 1) == Some(&b'"') {
        i + 1
    } else {
        return None;
    };
    let mut j = start + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'"' => return Some(j + 1),
            _ => j += 1,
        }
    }
    Some(bytes.len())
}

/// The content between the braces at `open`. The MIR printer doubles the
/// braces of an aggregate constant (`P {{ x: 1_i64 }}`), so a leading `{{` means
/// the real group starts one character later.
fn braced_inner(s: &str, open: usize) -> Option<&str> {
    if s[open..].starts_with("{{") {
        let close = matching(s, open + 1, '{', '}')?;
        Some(&s[open + 2..close])
    } else {
        let close = matching(s, open, '{', '}')?;
        Some(&s[open + 1..close])
    }
}

/// Split a comma-separated list at the top nesting level.
fn split_top_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = string_end(bytes, i) {
            cur.push_str(&s[i..end]);
            i = end;
            continue;
        }
        let c = bytes[i] as char;
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
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

/// Split at the top-level `;` (for `[value; count]` and `[T; N]`).
fn split_top_semi(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = string_end(bytes, i) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b';' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
        i += 1;
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
            Operand::Fmt { .. } => o.clone(),
        }
    }

    fn op_ret(&self, o: &Operand, dest_local: u32) -> Operand {
        match o {
            Operand::Copy(p) => Operand::Copy(self.place_ret(p, dest_local)),
            Operand::Move(p) => Operand::Move(self.place_ret(p, dest_local)),
            Operand::Const(c) => Operand::Const(self.const_(c)),
            Operand::Fmt { .. } => o.clone(),
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
        // Every operand and place goes through the `_ret` variants: the callee's
        // return place `_0` is a real local that the body both reads and writes
        // (an accumulator such as `_0 = Add(copy _0, move _5)`), and it must
        // become the caller's destination everywhere it appears, not just in
        // the final `Use`. Reading it as a plain local would alias the caller's
        // own `_0`.
        match rv {
            Rvalue::Use(o) => Rvalue::Use(self.op_ret(o, dest_local)),
            Rvalue::Binary(op, a, b) => {
                Rvalue::Binary(*op, self.op_ret(a, dest_local), self.op_ret(b, dest_local))
            }
            Rvalue::Unary(op, a) => Rvalue::Unary(*op, self.op_ret(a, dest_local)),
            Rvalue::Cast { from, to, op, kind } => Rvalue::Cast {
                from: from.clone(),
                to: to.clone(),
                op: self.op_ret(op, dest_local),
                kind: *kind,
            },
            Rvalue::Ref { mutbl, place } => {
                Rvalue::Ref { mutbl: *mutbl, place: self.place_ret(place, dest_local) }
            }
            Rvalue::RawRef { mutbl, place } => {
                Rvalue::RawRef { mutbl: *mutbl, place: self.place_ret(place, dest_local) }
            }
            Rvalue::Repeat { op, count } => {
                Rvalue::Repeat { op: self.op_ret(op, dest_local), count: *count }
            }
            Rvalue::Array(ops) => {
                Rvalue::Array(ops.iter().map(|o| self.op_ret(o, dest_local)).collect())
            }
            Rvalue::Aggregate(ops) => {
                Rvalue::Aggregate(ops.iter().map(|o| self.op_ret(o, dest_local)).collect())
            }
            Rvalue::Enum { name, ops } => Rvalue::Enum {
                name: name.clone(),
                ops: ops.iter().map(|o| self.op_ret(o, dest_local)).collect(),
            },
            Rvalue::Len(p) => Rvalue::Len(self.place_ret(p, dest_local)),
            Rvalue::WithOverflow { op, a, b } => Rvalue::WithOverflow {
                op: *op,
                a: self.op_ret(a, dest_local),
                b: self.op_ret(b, dest_local),
            },
            Rvalue::Discriminant(p) => Rvalue::Discriminant(self.place_ret(p, dest_local)),
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
            Terminator::Drop { place, target } => Terminator::Drop {
                place: self.place_ret(place, dest_local),
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
        if let Some(end) = string_end(bytes, i) {
            i = end;
            continue;
        }
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
/// Move a loop's test to the bottom of the loop.
///
/// A `while` loop is emitted as `header(test) -> body -> header`. Every
/// iteration therefore ends with an unconditional jump back to the header,
/// which the layout then cannot avoid. Copying the header's test to a block
/// after the body lets the latch fall through into the test, so an iteration
/// ends with a single conditional branch. The original header keeps its test and
/// runs once before the loop, which preserves `while` semantics: a loop whose
/// condition is false on entry still exits without running the body.
pub fn rotate_loops(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.rotate_loops();
    }
}

impl Function {
    fn rotate_loops(&mut self) {
        for _ in 0..16 {
            let n = self.blocks.len();
            let succs: Vec<Vec<u32>> = self.blocks.iter().map(|b| successors(&b.term)).collect();
            let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
            for (b, ss) in succs.iter().enumerate() {
                for &s in ss {
                    if !preds[s as usize].contains(&(b as u32)) {
                        preds[s as usize].push(b as u32);
                    }
                }
            }
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
            // Dominators, by the usual iterative intersection over the
            // predecessors. A back edge is an edge whose head dominates its
            // tail, which is what distinguishes a loop's own latch from the
            // block that merely feeds the loop.
            let mut dom: Vec<HashSet<usize>> = vec![HashSet::new(); n];
            for b in 0..n {
                if b == 0 {
                    dom[b].insert(0);
                } else {
                    dom[b].extend(0..n);
                }
            }
            loop {
                let mut changed = false;
                for b in 1..n {
                    let mut new: HashSet<usize> = HashSet::new();
                    if let Some((first, rest)) = preds[b].split_first() {
                        new = dom[*first as usize].clone();
                        for p in rest {
                            new.retain(|x| dom[*p as usize].contains(x));
                        }
                    }
                    new.insert(b);
                    if new != dom[b] {
                        dom[b] = new;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }

            let mut rotated = false;
            for h in 0..n {
                if !matches!(self.blocks[h].term, Terminator::SwitchInt { .. }) {
                    continue;
                }
                let backs: Vec<u32> = preds[h]
                    .iter()
                    .copied()
                    .filter(|&u| u != h as u32 && dom[u as usize].contains(&h))
                    .collect();
                if backs.len() != 1 {
                    continue;
                }
                let latch = backs[0];
                // The loop is the blocks the header dominates that can still
                // reach the latch. An edge into the header from outside that set
                // is the loop's entry, so the test has not moved yet.
                let in_loop = |b: usize| dom[b].contains(&h) && reach[b][latch as usize];
                if !preds[h].iter().any(|&p| !in_loop(p as usize)) {
                    continue;
                }
                let test = self.blocks[h].clone();
                self.blocks.push(test);
                let t = (self.blocks.len() - 1) as u32;
                redirect_target(&mut self.blocks[latch as usize].term, h as u32, t);
                rotated = true;
                break;
            }
            if !rotated {
                break;
            }
        }
    }
}

/// A natural loop: its header, the blocks it contains, and the preheader that
/// falls into it (when it has exactly one entry).
struct LoopInfo {
    header: u32,
    blocks: HashSet<u32>,
    preheader: Option<u32>,
}

/// Combine the invariant addends of a loop-carried accumulator.
///
/// `acc = acc + a + b + c` compiles to a chain of adds in the loop body, one per
/// addend. When the addends are invariant (computed outside the loop), summing
/// them once in the preheader turns the body into a single `acc += sum`. The AST
/// backend reaches this form through inlining and constant folding; doing it
/// here lets a `rustc` front end produce the same tight loop.
pub fn fold_accumulators(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.fold_accumulators();
    }
}

/// Fold a counted loop into its closed form.
///
/// `for _ in 0..n { acc += K }`, with `K` loop-invariant, is `acc += K * n`.
/// rustc's MIR keeps the loop (LLVM folds it later through scalar evolution),
/// so the rmc back end would run the body N times; the `refs` benchmark turns a
/// constant sum into N additions this way. The fold is exact only under wrapping
/// arithmetic, so it is skipped when overflow checks are on: a checked `+` would
/// trap at a different iteration than the folded `*`.
pub fn close_counted_loops(prog: &mut Program, overflow_checks: bool) {
    if overflow_checks {
        return;
    }
    for f in &mut prog.funcs {
        for _ in 0..8 {
            if !f.close_one_counted_loop() {
                break;
            }
        }
    }
}

/// The pieces of a loop that `close_counted_loops` matched: an induction
/// variable stepping by one from zero, an accumulator adding a loop-invariant
/// value each iteration, and the bound the induction variable is compared with.
struct CountedLoop {
    exit: u32,
    body: u32,
    acc: u32,
    bound: Operand,
    k: Operand,
}

impl Function {
    /// Match one `for i in 0..bound { acc += K }` loop and replace its body with
    /// `acc += K * bound`, guarded by the loop's own `0 < bound` test.
    fn close_one_counted_loop(&mut self) -> bool {
        let n = self.blocks.len();
        if n == 0 {
            return false;
        }
        let succs: Vec<Vec<u32>> = self.blocks.iter().map(|b| successors(&b.term)).collect();
        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (b, ss) in succs.iter().enumerate() {
            for &s in ss {
                if !preds[s as usize].contains(&(b as u32)) {
                    preds[s as usize].push(b as u32);
                }
            }
        }
        let mut dom: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        for b in 0..n {
            if b == 0 {
                dom[b].insert(0);
            } else {
                dom[b].extend(0..n);
            }
        }
        loop {
            let mut changed = false;
            for b in 1..n {
                let mut new: HashSet<usize> = HashSet::new();
                if let Some((first, rest)) = preds[b].split_first() {
                    new = dom[*first as usize].clone();
                    for p in rest {
                        new.retain(|x| dom[*p as usize].contains(x));
                    }
                }
                new.insert(b);
                if new != dom[b] {
                    dom[b] = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let Some(cl) = self.match_counted_loop(&succs, &preds, &dom, n) else {
            return false;
        };
        self.apply_counted_loop(cl);
        true
    }

    fn match_counted_loop(
        &self,
        succs: &[Vec<u32>],
        preds: &[Vec<u32>],
        dom: &[HashSet<usize>],
        n: usize,
    ) -> Option<CountedLoop> {
        for h in 0..n {
            let Terminator::SwitchInt { discr, targets, otherwise } = &self.blocks[h].term else {
                continue;
            };
            let (targets, otherwise) = (targets.clone(), *otherwise);
            // The back edge is a predecessor the header dominates; its tail is
            // the latch. Exactly one such edge keeps the shape a simple loop.
            let latch: Vec<u32> = preds[h]
                .iter()
                .copied()
                .filter(|&u| u != h as u32 && dom[u as usize].contains(&h))
                .collect();
            if latch.len() != 1 {
                continue;
            }
            // The loop is the latch plus every block that can reach it without
            // passing through the header. Following predecessors with the header
            // removed is what keeps the exit block out: the exit can only reach
            // the latch by going back through the header.
            let mut in_loop = vec![false; n];
            in_loop[h] = true;
            let mut stack = vec![latch[0]];
            while let Some(x) = stack.pop() {
                if in_loop[x as usize] {
                    continue;
                }
                in_loop[x as usize] = true;
                for &p in &preds[x as usize] {
                    if p != h as u32 {
                        stack.push(p);
                    }
                }
            }
            // A single entry from outside is the preheader.
            let outside: Vec<u32> =
                preds[h].iter().copied().filter(|&p| !in_loop[p as usize]).collect();
            if outside.len() != 1 {
                continue;
            }
            // One successor inside the loop is the body; one outside is the exit.
            let inner: Vec<u32> =
                succs[h].iter().copied().filter(|&s| in_loop[s as usize]).collect();
            let outer: Vec<u32> =
                succs[h].iter().copied().filter(|&s| !in_loop[s as usize]).collect();
            if inner.len() != 1 || outer.len() != 1 {
                continue;
            }
            let (body, exit) = (inner[0], outer[0]);
            // The condition must select the body on *true* and the exit on
            // *false*, i.e. `[0: exit, otherwise: body]`. The rewrite runs the
            // body once and jumps to the exit, which is only the loop's meaning
            // when a true comparison enters the body; a reversed
            // `[1: exit, otherwise: body]` (or any nonzero arm) would otherwise
            // terminate a loop that should continue.
            if targets.iter().any(|(v, t)| *v != 0 && *t != body) {
                continue;
            }
            if targets.iter().any(|(v, t)| *v == 0 && *t != exit) {
                continue;
            }
            if otherwise != body {
                continue;
            }
            // A single-block body that jumps straight back to the header.
            let body_blocks: Vec<u32> =
                (0..n as u32).filter(|&b| b != h as u32 && in_loop[b as usize]).collect();
            if body_blocks != vec![body]
                || !matches!(self.blocks[body as usize].term, Terminator::Goto(t) if t == h as u32)
            {
                continue;
            }
            // The test must be `iv < bound`, with `iv` an induction variable.
            let Some((op, a, b)) = self.bool_def(discr) else { continue };
            if op != BinOp::Lt {
                continue;
            }
            let (Operand::Copy(ivp) | Operand::Move(ivp)) = &a else { continue };
            if !ivp.proj.is_empty() {
                continue;
            }
            let iv = ivp.local;
            let bound = as_copy(&b);
            // `iv` must start at zero, outside the loop.
            if self.loop_iv_init(iv, &in_loop, n) != Some(0) {
                continue;
            }
            // Locals the loop writes: an invariant operand must not be one.
            let mut defined_in_loop: HashSet<u32> = HashSet::new();
            for b in 0..n {
                if !in_loop[b] {
                    continue;
                }
                for s in &self.blocks[b].stmts {
                    if let Stmt::Assign(d, _) = s {
                        defined_in_loop.insert(d.local);
                    }
                }
                if let Terminator::Call { dest, .. } = &self.blocks[b].term {
                    defined_in_loop.insert(dest.local);
                }
            }
            let invariant = |op: &Operand| {
                other_place(op).is_some_and(|l| !defined_in_loop.contains(&l))
            };
            // The header runs exactly once after the rewrite, so it must be no
            // more than the test: a local it writes and that is read again
            // outside the header would lose its per-iteration value, and a store
            // through a pointer is a side effect that must not be dropped either.
            // This includes the discriminant local: when the exit reads the
            // condition, the closed form's missing final failing test would
            // leave the last *true* value behind, so a condition observed
            // outside the header blocks the rewrite.
            let mut header_ok = true;
            for s in &self.blocks[h].stmts {
                match s {
                    Stmt::Nop => {}
                    Stmt::Assign(d, _) if d.proj.is_empty() => {
                        if self.local_read_elsewhere(d.local, h as u32, n) {
                            header_ok = false;
                            break;
                        }
                    }
                    _ => {
                        header_ok = false;
                        break;
                    }
                }
            }
            if !header_ok {
                continue;
            }
            // The body is exactly `acc += K` and `iv += 1`.
            let mut acc = None;
            let mut iv_updates = 0u32;
            let mut only_updates = true;
            for s in &self.blocks[body as usize].stmts {
                match s {
                    Stmt::Nop => {}
                    Stmt::Assign(d, Rvalue::Binary(BinOp::Add, x, y)) if d.proj.is_empty() => {
                        if let Some(c) = const_operand(y) {
                            // `iv += 1`.
                            if d.local == iv && matches!(other_place(x), Some(p) if p == iv) && c == 1 {
                                iv_updates += 1;
                                continue;
                            }
                        }
                        // `acc += K` (either operand order), K invariant.
                        for (self_side, other) in [(x, y), (y, x)] {
                            if matches!(other_place(self_side), Some(p) if p == d.local)
                                && invariant(other)
                            {
                                if acc.is_some() {
                                    only_updates = false;
                                }
                                acc = Some((d.local, as_copy(other)));
                                break;
                            }
                        }
                    }
                    _ => only_updates = false,
                }
            }
            // The induction variable must advance by exactly one each iteration:
            // `acc += K * bound` counts `bound` iterations, which matches a step
            // of one and nothing else.
            if !only_updates || iv_updates != 1 {
                continue;
            }
            let Some((acc, k)) = acc else { continue };
            if acc == iv
                || other_place(&k) == Some(acc)
                || other_place(&bound) == Some(acc)
                || !invariant(&bound)
            {
                continue;
            }
            // `iv` must be dead after the loop, or the closed form's missing
            // final value would be observable.
            if self.local_read_outside(iv, &in_loop, outside[0], n) {
                continue;
            }
            return Some(CountedLoop { exit, body, acc, bound, k });
        }
        None
    }

    fn apply_counted_loop(&mut self, cl: CountedLoop) {
        let t = self.fresh_local();
        let mul = Stmt::Assign(
            Place { local: t, proj: Vec::new() },
            Rvalue::Binary(BinOp::Mul, cl.k, cl.bound),
        );
        let add = Stmt::Assign(
            Place { local: cl.acc, proj: Vec::new() },
            Rvalue::Binary(
                BinOp::Add,
                Operand::Copy(Place { local: cl.acc, proj: Vec::new() }),
                Operand::Copy(Place { local: t, proj: Vec::new() }),
            ),
        );
        // The header keeps its `0 < bound` test; running the body once now does
        // the whole accumulation, and the body exits instead of looping.
        self.blocks[cl.body as usize].stmts = vec![mul, add];
        self.blocks[cl.body as usize].term = Terminator::Goto(cl.exit);
    }

    /// The constant `local` is initialised to outside the loop at `header`, if
    /// there is exactly one such definition and it is a literal.
    fn loop_iv_init(&self, local: u32, in_loop: &[bool], n: usize) -> Option<i64> {
        let mut init = None;
        for b in 0..n {
            if in_loop[b] {
                continue;
            }
            for s in &self.blocks[b].stmts {
                if let Stmt::Assign(d, Rvalue::Use(c)) = s {
                    if d.local == local && d.proj.is_empty() {
                        if init.is_some() {
                            return None;
                        }
                        init = const_operand(c);
                    }
                }
            }
        }
        init
    }

    /// Whether `local` is read anywhere except in block `h` itself (its own
    /// definitions do not count). Decides whether a header statement the
    /// loop-closing rewrite deletes was observable elsewhere.
    fn local_read_elsewhere(&self, local: u32, h: u32, n: usize) -> bool {
        for b in 0..n {
            if b as u32 == h {
                continue;
            }
            for s in &self.blocks[b].stmts {
                let mut found = false;
                if let Stmt::Assign(_, rv) = s {
                    scan_rv_operands(rv, &mut |op| {
                        if other_place(op) == Some(local) {
                            found = true;
                        }
                    });
                }
                if found {
                    return true;
                }
            }
            match &self.blocks[b].term {
                Terminator::SwitchInt { discr, .. } => {
                    if other_place(discr) == Some(local) {
                        return true;
                    }
                }
                Terminator::Assert { cond, .. } => {
                    if other_place(cond) == Some(local) {
                        return true;
                    }
                }
                Terminator::Call { args, .. } => {
                    if args.iter().any(|a| other_place(a) == Some(local)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Whether `local` is read anywhere outside the loop (its own definitions
    /// do not count). A read outside means the loop's final induction value is
    /// live, so the loop cannot be closed.
    fn local_read_outside(&self, local: u32, in_loop: &[bool], preheader: u32, n: usize) -> bool {
        for b in 0..n {
            // A rotated loop repeats the header test in the preheader, which
            // reads the induction variable at its initial value. That read is
            // still correct once the loop is closed, so it does not count.
            if in_loop[b] || b as u32 == preheader {
                continue;
            }
            for s in &self.blocks[b].stmts {
                let mut found = false;
                if let Stmt::Assign(_, rv) = s {
                    scan_rv_operands(rv, &mut |op| {
                        if other_place(op) == Some(local) {
                            found = true;
                        }
                    });
                }
                if found {
                    return true;
                }
            }
            match &self.blocks[b].term {
                Terminator::SwitchInt { discr, .. } => {
                    if other_place(discr) == Some(local) {
                        return true;
                    }
                }
                Terminator::Assert { cond, .. } => {
                    if other_place(cond) == Some(local) {
                        return true;
                    }
                }
                Terminator::Call { args, .. } => {
                    if args.iter().any(|a| other_place(a) == Some(local)) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// The comparison a boolean local holds: the `Binary(cmp, a, b)` that defines
    /// it. A rotated loop emits the same test twice (once in the preheader, once
    /// in the header), so identical definitions are accepted; disagreeing ones
    /// are not.
    fn bool_def(&self, op: &Operand) -> Option<(BinOp, Operand, Operand)> {
        let (Operand::Copy(p) | Operand::Move(p)) = op else { return None };
        if !p.proj.is_empty() {
            return None;
        }
        let mut found = None;
        for b in &self.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(d, Rvalue::Binary(o, a, c)) = s {
                    if d.local == p.local && d.proj.is_empty() && is_cmp_op(*o) {
                        if found.is_some() && found != Some((*o, a.clone(), c.clone())) {
                            return None;
                        }
                        found = Some((*o, a.clone(), c.clone()));
                    }
                }
            }
        }
        found
    }
}

/// The whole-local a place operand names, if any.
fn other_place(op: &Operand) -> Option<u32> {
    match op {
        Operand::Copy(p) | Operand::Move(p) if p.proj.is_empty() => Some(p.local),
        _ => None,
    }
}

/// Force an operand to a copy: a `Move` becomes a `Copy`, so the operand can be
/// read again without being consumed.
fn as_copy(op: &Operand) -> Operand {
    match op {
        Operand::Move(p) => Operand::Copy(p.clone()),
        other => other.clone(),
    }
}

fn const_operand(op: &Operand) -> Option<i64> {
    match op {
        Operand::Const(Const::Int(n)) => Some(*n),
        _ => None,
    }
}

fn is_cmp_op(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// A rewrite the accumulator fold will apply, found while borrowing the blocks
/// immutably and applied afterwards.
struct AccFold {
    preheader: u32,
    final_site: (usize, usize),
    chain_sites: Vec<(usize, usize)>,
    acc: u32,
    addends: Vec<u32>,
}

impl Function {
    fn fold_accumulators(&mut self) {
        for _ in 0..8 {
            if !self.fold_one_accumulator() {
                break;
            }
        }
    }

    fn fold_one_accumulator(&mut self) -> bool {
        let loops = self.natural_loops();
        let uses = self.use_counts();
        let fold = self.find_acc_fold(&loops, &uses);
        let Some(fold) = fold else { return false };
        self.apply_acc_fold(fold);
        true
    }

    fn find_acc_fold(&self, loops: &[LoopInfo], uses: &HashMap<u32, u32>) -> Option<AccFold> {
        // Where each whole local is defined, as `(block, statement index)`.
        let mut def_sites: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
        for (b, blk) in self.blocks.iter().enumerate() {
            for (i, s) in blk.stmts.iter().enumerate() {
                if let Stmt::Assign(d, _) = s {
                    if d.proj.is_empty() {
                        def_sites.entry(d.local).or_default().push((b, i));
                    }
                }
            }
        }
        for lp in loops {
            let Some(preheader) = lp.preheader else { continue };
            if !matches!(self.blocks[preheader as usize].term, Terminator::Goto(t) if t == lp.header) {
                continue;
            }
            let mut defined_in: HashSet<u32> = HashSet::new();
            for &b in &lp.blocks {
                for s in &self.blocks[b as usize].stmts {
                    if let Stmt::Assign(d, _) = s {
                        if d.proj.is_empty() {
                            defined_in.insert(d.local);
                        }
                    }
                }
                if let Terminator::Call { dest, .. } = &self.blocks[b as usize].term {
                    if dest.proj.is_empty() {
                        defined_in.insert(dest.local);
                    }
                }
            }
            let invariant = |l: u32| !defined_in.contains(&l) && l != 0;
            // A local continues the chain when it is defined inside the loop by
            // one more `add`.
            let chain_def = |l: u32| -> Option<(usize, usize)> {
                let v = def_sites.get(&l)?;
                if v.len() != 1 {
                    return None;
                }
                let (b, i) = v[0];
                if !lp.blocks.contains(&(b as u32)) {
                    return None;
                }
                matches!(self.blocks[b].stmts[i], Stmt::Assign(_, Rvalue::Binary(BinOp::Add, _, _)))
                    .then_some((b, i))
            };
            let is_link = |l: u32| chain_def(l).is_some();

            for b in 0..self.blocks.len() {
                if !lp.blocks.contains(&(b as u32)) {
                    continue;
                }
                for i in 0..self.blocks[b].stmts.len() {
                    let Stmt::Assign(dest, Rvalue::Binary(BinOp::Add, x, y)) =
                        &self.blocks[b].stmts[i]
                    else {
                        continue;
                    };
                    if !dest.proj.is_empty() {
                        continue;
                    }
                    let acc = dest.local;
                    if def_sites
                        .get(&acc)
                        .map(|v| v.iter().filter(|(bb, _)| lp.blocks.contains(&(*bb as u32))).count())
                        != Some(1)
                    {
                        continue;
                    }
                    for (chain, first) in [(x, y), (y, x)] {
                        let (Operand::Copy(cp) | Operand::Move(cp)) = chain else { continue };
                        if !cp.proj.is_empty() {
                            continue;
                        }
                        let start = cp.local;
                        if start != acc && !is_link(start) {
                            continue;
                        }
                        let (Operand::Copy(fp) | Operand::Move(fp)) = first else { continue };
                        if !fp.proj.is_empty() {
                            continue;
                        }
                        let mut addends = vec![fp.local];
                        let mut chain_sites: Vec<(usize, usize)> = vec![(b, i)];
                        let mut cur = start;
                        let mut ok = true;
                        let mut steps = 0;
                        while cur != acc {
                            steps += 1;
                            if steps > 64 {
                                ok = false;
                                break;
                            }
                            let Some((cb, ci)) = chain_def(cur) else { ok = false; break };
                            if uses.get(&cur).copied().unwrap_or(0) != 1 {
                                ok = false;
                                break;
                            }
                            let Stmt::Assign(_, Rvalue::Binary(BinOp::Add, l, r)) =
                                &self.blocks[cb].stmts[ci]
                            else {
                                ok = false;
                                break;
                            };
                            // One operand continues the chain; the other is an
                            // addend. Require a unique choice.
                            let mut choices = Vec::new();
                            for (side, other) in [(l, r), (r, l)] {
                                let (Operand::Copy(sp) | Operand::Move(sp)) = side else { continue };
                                let (Operand::Copy(op) | Operand::Move(op)) = other else { continue };
                                if !op.proj.is_empty() {
                                    continue;
                                }
                                if sp.local == acc || is_link(sp.local) {
                                    choices.push((sp.local, op.local));
                                }
                            }
                            if choices.len() != 1 {
                                ok = false;
                                break;
                            }
                            let (next, addend) = choices[0];
                            addends.push(addend);
                            chain_sites.push((cb, ci));
                            cur = next;
                        }
                        if !ok || addends.len() < 2 {
                            continue;
                        }
                        if !addends
                            .iter()
                            .all(|l| invariant(*l) && !self.local_ty(*l).is_aggregate())
                        {
                            continue;
                        }
                        return Some(AccFold {
                            preheader,
                            final_site: (b, i),
                            chain_sites,
                            acc,
                            addends,
                        });
                    }
                }
            }
        }
        None
    }

    fn apply_acc_fold(&mut self, fold: AccFold) {
        // Sum the addends in the preheader, one add at a time.
        let mut prev = fold.addends[0];
        for &a in fold.addends.iter().skip(1) {
            let target = self.fresh_local();
            self.blocks[fold.preheader as usize].stmts.push(Stmt::Assign(
                Place { local: target, proj: Vec::new() },
                Rvalue::Binary(
                    BinOp::Add,
                    Operand::Copy(Place { local: prev, proj: Vec::new() }),
                    Operand::Copy(Place { local: a, proj: Vec::new() }),
                ),
            ));
            prev = target;
        }
        let sum = prev;
        // The final site becomes `acc += sum`; the rest of the chain is dropped.
        let (fb, fi) = fold.final_site;
        self.blocks[fb].stmts[fi] = Stmt::Assign(
            Place { local: fold.acc, proj: Vec::new() },
            Rvalue::Binary(
                BinOp::Add,
                Operand::Copy(Place { local: fold.acc, proj: Vec::new() }),
                Operand::Copy(Place { local: sum, proj: Vec::new() }),
            ),
        );
        let drop: HashSet<(usize, usize)> =
            fold.chain_sites.iter().copied().filter(|s| *s != fold.final_site).collect();
        for (b, blk) in self.blocks.iter_mut().enumerate() {
            if drop.is_empty() {
                break;
            }
            let mut k = 0;
            let mut kept = Vec::with_capacity(blk.stmts.len());
            for s in std::mem::take(&mut blk.stmts) {
                if !drop.contains(&(b, k)) {
                    kept.push(s);
                }
                k += 1;
            }
            blk.stmts = kept;
        }
    }

    fn fresh_local(&mut self) -> u32 {
        let l = self.locals.len() as u32;
        self.locals.push(LocalDecl { ty: Box::new(Ty::I64), is_arg: false, name: None });
        l
    }

    /// Every natural loop in the function, with the preheader when it has
    /// exactly one entry.
    fn natural_loops(&self) -> Vec<LoopInfo> {
        let n = self.blocks.len();
        let succ: Vec<Vec<u32>> = self.blocks.iter().map(|b| successors(&b.term)).collect();
        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (b, ss) in succ.iter().enumerate() {
            for &s in ss {
                if !preds[s as usize].contains(&(b as u32)) {
                    preds[s as usize].push(b as u32);
                }
            }
        }
        let mut reach: Vec<Vec<bool>> = vec![vec![false; n]; n];
        for b in 0..n {
            reach[b][b] = true;
            let mut stack = vec![b];
            while let Some(x) = stack.pop() {
                for &s in &succ[x] {
                    if !reach[b][s as usize] {
                        reach[b][s as usize] = true;
                        stack.push(s as usize);
                    }
                }
            }
        }
        let mut dom: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        for b in 0..n {
            if b == 0 {
                dom[b].insert(0);
            } else {
                dom[b].extend(0..n);
            }
        }
        loop {
            let mut changed = false;
            for b in 1..n {
                let mut new: HashSet<usize> = HashSet::new();
                if let Some((first, rest)) = preds[b].split_first() {
                    new = dom[*first as usize].clone();
                    for p in rest {
                        new.retain(|x| dom[*p as usize].contains(x));
                    }
                }
                new.insert(b);
                if new != dom[b] {
                    dom[b] = new;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut loops = Vec::new();
        for h in 0..n {
            let backs: Vec<u32> = preds[h]
                .iter()
                .copied()
                .filter(|&u| u != h as u32 && dom[u as usize].contains(&h))
                .collect();
            if backs.len() != 1 {
                continue;
            }
            let latch = backs[0];
            let in_loop = |b: usize| dom[b].contains(&h) && reach[b][latch as usize];
            let mut blocks: HashSet<u32> = HashSet::new();
            for b in 0..n {
                if in_loop(b) {
                    blocks.insert(b as u32);
                }
            }
            let outside: Vec<u32> = preds[h]
                .iter()
                .copied()
                .filter(|&p| !in_loop(p as usize))
                .collect();
            let preheader = if outside.len() == 1 { Some(outside[0]) } else { None };
            loops.push(LoopInfo { header: h as u32, blocks, preheader });
        }
        loops
    }
}

pub fn layout_blocks(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.layout_blocks();
    }
}

/// Replace every local whose only definition is an inlined call result with the
/// call destination, so the value has one definition and one name.
///
/// Inlining a call keeps its destination local and copies the result into the
/// caller's own temporary (`_16 = _44; _17 = _16; ...`), because the inlined
/// body ends in `goto` to the call's target, which needs a return slot. That
/// copy chain both hides the value from promotion (the local is written more
/// than once) and forces it through memory. Rewriting the readers of the
/// destination to name it directly removes the copy and lets the value be
/// promoted like any other single-definition temporary.
pub fn coalesce_call_results(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.coalesce_call_results();
    }
}

impl Function {
    fn coalesce_call_results(&mut self) {
        // The definition count of every local, from statements and calls.
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
        // Every call destination: a chain source must be one, so the value is a
        // call result rather than a re-definable local.
        let mut call_dest: HashSet<u32> = HashSet::new();
        for b in &self.blocks {
            if let Terminator::Call { dest, .. } = &b.term {
                if dest.proj.is_empty() {
                    call_dest.insert(dest.local);
                }
            }
        }
        // How many times each local is read.
        let mut uses: HashMap<u32, u32> = HashMap::new();
        for b in &self.blocks {
            for s in &b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    scan_rv_operands(rv, &mut |op| {
                        if let Operand::Copy(p) | Operand::Move(p) = op {
                            *uses.entry(p.local).or_insert(0) += 1;
                        }
                    });
                }
            }
            match &b.term {
                Terminator::SwitchInt { discr, .. } | Terminator::Assert { cond: discr, .. } => {
                    if let Operand::Copy(p) | Operand::Move(p) = discr {
                        *uses.entry(p.local).or_insert(0) += 1;
                    }
                }
                Terminator::Call { args, .. } => {
                    for a in args {
                        if let Operand::Copy(p) | Operand::Move(p) = a {
                            *uses.entry(p.local).or_insert(0) += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        // `src` is a call result copied once into `dest`; rewrite readers of
        // `dest` to name `src`, and drop the copy.
        let mut cand: HashMap<u32, u32> = HashMap::new();
        for b in &self.blocks {
            for s in &b.stmts {
                let Stmt::Assign(dest, Rvalue::Use(Operand::Copy(src) | Operand::Move(src))) = s
                else {
                    continue;
                };
                if !dest.proj.is_empty() || !src.proj.is_empty() || dest.local == src.local {
                    continue;
                }
                // The copy must be the source's only reader, and the source must
                // be a call destination, so the value at the copy is the call's
                // result. The call counts once in `defs`, so a source defined
                // once is the call alone; a second definition is a write whose
                // position cannot change the call's result at the copy.
                if !call_dest.contains(&src.local) {
                    continue;
                }
                if uses.get(&src.local).copied().unwrap_or(0) != 1 {
                    continue;
                }
                if !(1..=2).contains(&defs.get(&src.local).copied().unwrap_or(0)) {
                    continue;
                }
                if defs.get(&dest.local).copied().unwrap_or(0) != 1 {
                    continue;
                }
                cand.insert(dest.local, src.local);
            }
        }
        if cand.is_empty() {
            return;
        }
        let rewrite = |p: &mut Place| {
            if p.proj.is_empty() {
                if let Some(&to) = cand.get(&p.local) {
                    p.local = to;
                }
            }
        };
        for b in &mut self.blocks {
            for s in &mut b.stmts {
                if let Stmt::Assign(_, rv) = s {
                    for_each_operand_mut(rv, &mut |op| {
                        if let Operand::Copy(p) | Operand::Move(p) = op {
                            rewrite(p);
                        }
                    });
                }
            }
            match &mut b.term {
                Terminator::SwitchInt { discr, .. } => {
                    if let Operand::Copy(p) | Operand::Move(p) = discr {
                        rewrite(p);
                    }
                }
                Terminator::Assert { cond, .. } => {
                    if let Operand::Copy(p) | Operand::Move(p) = cond {
                        rewrite(p);
                    }
                }
                Terminator::Call { args, .. } => {
                    for a in args {
                        if let Operand::Copy(p) | Operand::Move(p) = a {
                            rewrite(p);
                        }
                    }
                }
                _ => {}
            }
        }
        // Drop the now-unused copies, so the call result keeps its single
        // definition and its register is written once.
        for b in &mut self.blocks {
            b.stmts.retain(|s| {
                !matches!(s, Stmt::Assign(d, Rvalue::Use(Operand::Copy(src) | Operand::Move(src)))
                    if d.proj.is_empty()
                        && src.proj.is_empty()
                        && cand.get(&d.local) == Some(&src.local))
            });
        }
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
                Terminator::Drop { target, .. } => *target = remap(*target),
                _ => {}
            }
        }
        self.blocks = kept;
    }
}

/// Rewrite a terminator so that every edge to `from` goes to `to` instead.
fn redirect_target(t: &mut Terminator, from: u32, to: u32) {
    match t {
        Terminator::Goto(b) => {
            if *b == from {
                *b = to;
            }
        }
        Terminator::SwitchInt { targets, otherwise, .. } => {
            for (_, b) in targets.iter_mut() {
                if *b == from {
                    *b = to;
                }
            }
            if *otherwise == from {
                *otherwise = to;
            }
        }
        Terminator::Call { target: Some(b), .. } => {
            if *b == from {
                *b = to;
            }
        }
        Terminator::Assert { target, .. } => {
            if *target == from {
                *target = to;
            }
        }
        Terminator::Drop { target, .. } => {
            if *target == from {
                *target = to;
            }
        }
        _ => {}
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
        Terminator::Drop { target, .. } => Some(*target),
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
                Terminator::Drop { target, .. } => *target = remap(*target),
                _ => {}
            }
        }
        self.blocks = kept;
    }
}

/// Drop the safety `assert`s rustc's `-O` MIR leaves behind.
///
/// rustc checks a division and a remainder for a zero divisor and for the
/// `i64::MIN / -1` overflow, and an array index against the length. Those
/// checks are statements in the MIR, so the backend would emit them even though
/// the surrounding code already rules them out — the divisor of `gcd`'s
/// remainder is the loop guard, and the index of a loop bounded by the array
/// length can never be out of range. rmc's own front end does not insert these
/// checks unless asked (`--overflow-checks`/`--bounds-checks`), so dropping them
/// here is what lets the two pipelines be compared on equal footing.
///
/// A check is dropped when the compared operand is a whole local whose
/// dominating definitions all make the comparison hold: for a `!= 0` guard the
/// local is defined by a constant or by a `%`/`/` whose own divisor is known
/// non-zero, and for a `< LEN` bounds guard the index is bounded by a loop
/// counter compared against `LEN`. Anything else keeps the check, so the pass
/// never removes one the program actually needs.
pub fn drop_redundant_asserts(prog: &mut Program) {
    for f in &mut prog.funcs {
        f.drop_redundant_asserts();
    }
}

/// The facts this pass tracks at a program point. Every field only ever loses
/// entries at a control-flow join, so the analysis is monotone and terminates.
#[derive(Clone, Default, PartialEq)]
struct AssertFacts {
    /// Locals proven non-zero on every path here.
    nonzero: HashSet<u32>,
    /// Locals proven equal to zero on every path here.
    zero: HashSet<u32>,
    /// `d = copy s` in the current block: `d` holds `s`'s value.
    alias: HashMap<u32, u32>,
    /// A bool `b = (x == 0)`: `b` is true exactly when `x` is zero.
    iszero: HashMap<u32, u32>,
}

impl Function {
    fn drop_redundant_asserts(&mut self) {
        let n = self.blocks.len();
        let mut facts_in: Vec<Option<AssertFacts>> = vec![None; n];
        facts_in[0] = Some(AssertFacts::default());
        loop {
            let mut changed = false;
            for b in 0..n {
                let Some(facts) = facts_at_term(&self.blocks[b], &facts_in[b]) else { continue };
                for (s, edge) in edges_with_facts(&self.blocks[b].term) {
                    let s = s as usize;
                    let mut contributed = facts.clone();
                    match edge {
                        EdgeFact::NonZero(l) => {
                            contributed.nonzero.insert(contributed.resolve(l));
                        }
                        EdgeFact::Zero(l) => {
                            contributed.zero.insert(contributed.resolve(l));
                        }
                        EdgeFact::None => {}
                    }
                    let merged = match &facts_in[s] {
                        None => Some(contributed),
                        Some(prev) => Some(prev.intersect(&contributed)),
                    };
                    if merged != facts_in[s] {
                        changed = true;
                    }
                    facts_in[s] = merged;
                }
            }
            if !changed {
                break;
            }
        }
        // Drop an `assert` the state at its own terminator proves. Two forms
        // matter: `assert(x)` with `x` known non-zero (rustc's direct form), and
        // the div-by-zero check `assert(!b)` where `b = (x == 0)` and `x` is
        // known non-zero (the folded form the backend emits as a `cmp`/`jne`).
        // The `bool` is defined by a statement in the same block, so the block's
        // statements are replayed onto the entry state first.
        for b in 0..n {
            let Some(facts) = facts_at_term(&self.blocks[b], &facts_in[b]) else { continue };
            let Terminator::Assert { cond, negate, target } = &self.blocks[b].term else {
                continue;
            };
            let Some(p) = operand_place(cond) else { continue };
            if !p.proj.is_empty() {
                continue;
            }
            let holds = if *negate {
                facts.zero.contains(&facts.resolve(p.local))
                    || facts
                        .iszero
                        .get(&p.local)
                        .is_some_and(|x| facts.nonzero.contains(&facts.resolve(*x)))
            } else {
                facts.nonzero.contains(&facts.resolve(p.local))
            };
            if holds {
                let target = *target;
                self.blocks[b].term = Terminator::Goto(target);
            }
        }
        // Dropping the assert leaves the `bool` it tested dead (`_6 = Eq(..)`),
        // and the backend would still materialise it. Remove assignments to a
        // whole local nothing reads, when the rvalue is free of side effects.
        self.drop_dead_pure_assignments();
    }

    fn drop_dead_pure_assignments(&mut self) {
        loop {
            let uses = self.use_counts();
            let mut changed = false;
            for b in &mut self.blocks {
                b.stmts.retain(|s| {
                    if let Stmt::Assign(d, rv) = s {
                        // `_0` is the return value, which `return` reads without
                        // naming it, so `use_counts` never sees that read.
                        if d.proj.is_empty()
                            && d.local != 0
                            && uses.get(&d.local).copied().unwrap_or(0) == 0
                            && rv_is_pure(rv)
                        {
                            changed = true;
                            return false;
                        }
                    }
                    true
                });
            }
            if !changed {
                break;
            }
        }
    }
}

/// The comparison rvalues: pure, so safe to drop when their `bool` destination
/// is unread. Only these are dropped — a wider set risks removing an assignment
/// whose operand the backend still needs (a moved pointer, say).
fn rv_is_pure(rv: &Rvalue) -> bool {
    matches!(
        rv,
        Rvalue::Binary(
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge,
            _,
            _,
        )
    )
}

impl AssertFacts {
    /// The local whose value `l` is known to share, following the copy chain.
    fn resolve(&self, l: u32) -> u32 {
        let mut cur = l;
        for _ in 0..self.alias.len() + 1 {
            match self.alias.get(&cur) {
                Some(next) if *next != cur => cur = *next,
                _ => break,
            }
        }
        cur
    }

    /// Invalidate every fact that depends on `l`.
    fn kill(&mut self, l: u32) {
        self.nonzero.remove(&l);
        self.zero.remove(&l);
        self.alias.remove(&l);
        self.alias.retain(|_, v| *v != l);
        self.iszero.remove(&l);
        self.iszero.retain(|_, v| *v != l);
    }

    fn intersect(&self, other: &AssertFacts) -> AssertFacts {
        AssertFacts {
            nonzero: self.nonzero.intersection(&other.nonzero).copied().collect(),
            zero: self.zero.intersection(&other.zero).copied().collect(),
            alias: self
                .alias
                .iter()
                .filter(|(d, s)| other.alias.get(d) == Some(s))
                .map(|(d, s)| (*d, *s))
                .collect(),
            iszero: self
                .iszero
                .iter()
                .filter(|(b, x)| other.iszero.get(b) == Some(x))
                .map(|(b, x)| (*b, *x))
                .collect(),
        }
    }
}

/// The facts after a block's statements run, given its entry facts. `None` when
/// the block is unreachable (no entry facts have reached it yet).
fn facts_at_term(block: &BasicBlock, entry: &Option<AssertFacts>) -> Option<AssertFacts> {
    let mut facts = entry.clone()?;
    // Locals written later in the block: an alias to one of them is not valid
    // by the time the terminator runs.
    let mut later_writes: HashSet<u32> = HashSet::new();
    for s in block.stmts.iter().rev() {
        if let Some(l) = written_local(s) {
            later_writes.insert(l);
        }
    }
    let mut killed: HashSet<u32> = HashSet::new();
    for s in &block.stmts {
        if let Some(l) = written_local(s) {
            facts.kill(l);
            killed.insert(l);
            later_writes.remove(&l);
        }
        if let Stmt::Assign(d, rv) = s {
            if d.proj.is_empty() {
                match rv {
                    Rvalue::Use(Operand::Const(Const::Int(v))) => {
                        if *v == 0 {
                            facts.zero.insert(d.local);
                        } else {
                            facts.nonzero.insert(d.local);
                        }
                    }
                    Rvalue::Use(Operand::Copy(s) | Operand::Move(s))
                        if s.proj.is_empty()
                            && !later_writes.contains(&s.local)
                            && !killed.contains(&s.local) =>
                    {
                        facts.alias.insert(d.local, s.local);
                    }
                    Rvalue::Binary(BinOp::Eq, a, b) => {
                        if let Some(x) = zero_eq_source(a, b) {
                            facts.iszero.insert(d.local, x);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Some(facts)
}

/// For `Eq(a, b)` where one operand is the constant `0`, the local the other
/// names — the source of a `x == 0` comparison.
fn zero_eq_source(a: &Operand, b: &Operand) -> Option<u32> {
    let is_zero = |o: &Operand| matches!(o, Operand::Const(Const::Int(0)));
    let whole = |o: &Operand| {
        operand_place(o).filter(|p| p.proj.is_empty()).map(|p| p.local)
    };
    if is_zero(b) {
        whole(a)
    } else if is_zero(a) {
        whole(b)
    } else {
        None
    }
}

/// The fact a control-flow edge proves about the `switchInt` discriminant.
#[derive(Clone, Copy)]
enum EdgeFact {
    None,
    NonZero(u32),
    Zero(u32),
}

/// Each successor of a terminator paired with the fact its edge proves.
///
/// `switchInt(d) -> [0: eq, otherwise: ne]` proves `d != 0` on the edge to `ne`
/// (it is only taken when the discriminant is none of the listed values) and
/// `d == 0` on the edge to `eq`. Every other edge proves nothing.
fn edges_with_facts(t: &Terminator) -> Vec<(u32, EdgeFact)> {
    match t {
        Terminator::SwitchInt { discr, targets, otherwise } => {
            let local = operand_place(discr).filter(|p| p.proj.is_empty()).map(|p| p.local);
            let mut v: Vec<(u32, EdgeFact)> = targets
                .iter()
                .map(|(val, b)| {
                    let fact = match (val, local) {
                        (0, Some(l)) => EdgeFact::Zero(l),
                        _ => EdgeFact::None,
                    };
                    (*b, fact)
                })
                .collect();
            let nonzero = if targets.iter().any(|(v, _)| *v == 0) {
                local.map(EdgeFact::NonZero)
            } else {
                None
            };
            v.push((*otherwise, nonzero.unwrap_or(EdgeFact::None)));
            v
        }
        Terminator::Goto(b) => vec![(*b, EdgeFact::None)],
        Terminator::Call { target, .. } => target.iter().map(|b| (*b, EdgeFact::None)).collect(),
        Terminator::Assert { target, .. } => vec![(*target, EdgeFact::None)],
        Terminator::Drop { target, .. } => vec![(*target, EdgeFact::None)],
        Terminator::Return | Terminator::Unreachable | Terminator::Abort => Vec::new(),
    }
}

fn operand_place(op: &Operand) -> Option<&Place> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => Some(p),
        Operand::Const(_) | Operand::Fmt { .. } => None,
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
    // Interprocedural purity: a user function whose body only computes a value
    // from its arguments (no store, no address-taking op, no call to a runtime
    // helper or another impure function) is side-effect-free, so a call to it
    // can be hoisted out of a loop. rustc leaves these accessor calls in place,
    // and `refs` leans on hoisting `Wide::total`/`pair` out of the loop.
    let pure_fns = pure_function_set(prog);
    // LICM is intraprocedural: hoisting inside one function never changes
    // another's blocks. So drive each function to its own fixpoint instead of
    // restarting the whole program whenever any function changes, which made
    // the pass O(rounds x functions) and dominated compile time on large input.
    for f in &mut prog.funcs {
        for _ in 0..LICM_ROUNDS {
            if !f.licm_once(&pure_fns) {
                break;
            }
        }
    }
}

/// The names of the user functions whose bodies are side-effect-free, computed
/// as a greatest fixpoint with a worklist: a function is impure if it stores to
/// memory, takes an address, makes an indirect call, calls an unknown/extern
/// function, or calls a function already known impure. Impurity propagates
/// backwards along the call graph, so a recursive or mutually recursive group
/// that only reads its arguments stays pure. Linear in the call graph, not a
/// quadratic re-scan.
fn pure_function_set(prog: &Program) -> HashSet<String> {
    let index: HashMap<&str, usize> =
        prog.funcs.iter().enumerate().map(|(i, f)| (f.name.as_str(), i)).collect();
    let n = prog.funcs.len();
    let mut impure = vec![false; n];
    // Reverse call edges: `callers[callee]` lists the functions that call it.
    let mut callers: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, f) in prog.funcs.iter().enumerate() {
        // A body-less declaration cannot be verified; treat it as impure.
        if f.blocks.is_empty() {
            impure[i] = true;
            continue;
        }
        for b in &f.blocks {
            if block_self_impure(b) {
                impure[i] = true;
            }
            if let Terminator::Call { func: CallTarget::Direct(name), .. } = &b.term {
                if pure_runtime_call(name) {
                    continue;
                }
                match index.get(name.as_str()) {
                    Some(&g) => callers[g].push(i),
                    // An unknown name is a runtime helper rmc cannot vouch for.
                    None => impure[i] = true,
                }
            }
        }
    }

    // Propagate impurity backwards from the initially-impure functions.
    let mut work: Vec<usize> = (0..n).filter(|&i| impure[i]).collect();
    while let Some(g) = work.pop() {
        for &caller in &callers[g] {
            if !impure[caller] {
                impure[caller] = true;
                work.push(caller);
            }
        }
    }

    prog.funcs
        .iter()
        .enumerate()
        .filter(|(i, _)| !impure[*i])
        .map(|(_, f)| f.name.clone())
        .collect()
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
    fn licm_once(&mut self, pure_fns: &HashSet<String>) -> bool {
        let nblocks = self.blocks.len();
        if nblocks == 0 {
            return false;
        }
        let defs = self.def_counts();
        let use_counts = self.use_counts();

        // Which blocks write memory (a store, a call, or an address-taking op).
        let writes_mem: Vec<bool> = self.blocks.iter().map(|b| block_writes_mem(b)).collect();
        let impure_call: Vec<bool> = self
            .blocks
            .iter()
            .map(|b| block_has_impure_call(b, pure_fns))
            .collect();

        // Predecessors and successors, computed once. The back-edge test below
        // needs "can `s` reach `b`", which a per-function reachability matrix
        // answers in O(1) instead of a fresh reverse walk per edge.
        let succ: Vec<Vec<u32>> = self.blocks.iter().map(|b| successors(&b.term)).collect();
        let mut preds: Vec<Vec<u32>> = vec![Vec::new(); nblocks];
        for (b, ss) in succ.iter().enumerate() {
            for &s in ss {
                if let Some(p) = preds.get_mut(s as usize) {
                    p.push(b as u32);
                }
            }
        }
        let mut reach: Vec<Vec<bool>> = vec![vec![false; nblocks]; nblocks];
        for s in 0..nblocks {
            reach[s][s] = true;
            let mut stack = vec![s];
            while let Some(x) = stack.pop() {
                for &t in &succ[x] {
                    if !reach[s][t as usize] {
                        reach[s][t as usize] = true;
                        stack.push(t as usize);
                    }
                }
            }
        }

        // Each back edge (an edge `p -> s` where `s` can reach `p`) is a latch;
        // group latches by their header. Numeric order is not enough: rustc can
        // lay a loop out so its preheader has a *lower* block number than the
        // header, which would misread that edge as a back edge.
        let mut latches: HashMap<u32, Vec<u32>> = HashMap::new();
        for (b, ss) in succ.iter().enumerate() {
            for &s in ss {
                if s == b as u32 || reach[s as usize][b] {
                    latches.entry(s).or_default().push(b as u32);
                }
            }
        }
        // Deterministic order so the output is stable across runs.
        let mut headers: Vec<u32> = latches.keys().copied().collect();
        headers.sort_unstable();

        let mut any_changed = false;
        for header in headers {
            if self.licm_loop(header, &latches, &preds, &defs, &use_counts, &writes_mem, &impure_call, pure_fns) {
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
        pure_fns: &HashSet<String>,
    ) -> bool {
        // The preheader is the header's only predecessor from *outside* the
        // loop: then the loop is entered from exactly one place, and hoisting to
        // it runs once. (The latches are predecessors too, but they are inside.)
        let loop_blocks = self.loop_blocks(header, latches, preds);
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
            if !call_is_pure(name, pure_fns) {
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
    fn loop_blocks(&self, header: u32, latches: &HashMap<u32, Vec<u32>>, preds: &[Vec<u32>]) -> HashSet<u32> {
        let mut in_loop: HashSet<u32> = HashSet::new();
        in_loop.insert(header);
        let mut worklist: Vec<u32> = latches.get(&header).cloned().unwrap_or_default();
        while let Some(b) = worklist.pop() {
            if !in_loop.insert(b) {
                continue;
            }
            // Walk predecessors, stopping at the header (its other predecessors
            // are outside the loop).
            for &p in &preds[b as usize] {
                if p != header {
                    worklist.push(p);
                }
            }
        }
        in_loop
    }
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
        Terminator::Drop { target, .. } => vec![*target],
        Terminator::Return | Terminator::Unreachable | Terminator::Abort => Vec::new(),
    }
}

/// True when a call's arguments are all invariant: each is a constant, or a
/// read of a whole local that the loop neither defines nor moves.
fn call_args_invariant(args: &[Operand], is_invariant: &impl Fn(u32) -> bool) -> bool {
    args.iter().all(|a| match a {
        Operand::Const(_) => true,
        Operand::Copy(p) | Operand::Move(p) => p.proj.is_empty() && is_invariant(p.local),
        Operand::Fmt { .. } => true,
    })
}

/// True when a block contains anything that could write memory: a store through
/// a projection (`*p = x`, `arr[i] = x`), an address-taking rvalue, or a call to
/// a function that is not known pure. A pure call is not a store, so it does not
/// block hoisting a load past it.
/// The runtime helpers that are side-effect-free: they read their arguments and
/// return a value, with no store and no syscall. A call to one is safe to hoist
/// out of a loop.
fn pure_runtime_call(name: &str) -> bool {
    matches!(
        name,
        "__rmc_imin"
            | "__rmc_imax"
            | "__rmc_umin"
            | "__rmc_umax"
            | "__rmc_compare_bytes"
            | "__rmc_str_len"
            | "__rmc_str_lt"
            | "__rmc_str_contains"
    )
}

/// Whether a direct call is side-effect-free: a known pure runtime leaf, or a
/// user function in the interprocedural purity set. Anything else (a runtime
/// helper that prints or allocates, an unknown name) is treated as impure.
fn call_is_pure(name: &str, pure_fns: &HashSet<String>) -> bool {
    pure_runtime_call(name) || pure_fns.contains(name)
}

fn block_writes_mem(b: &BasicBlock) -> bool {
    block_self_impure(b)
}

/// A statement or terminator that is a side effect *by itself*: a store, an
/// address-taking op, or an indirect call. A direct call is judged separately
/// (its callee may be pure).
fn block_self_impure(b: &BasicBlock) -> bool {
    b.stmts.iter().any(|s| match s {
        Stmt::Assign(dest, rv) => {
            !dest.proj.is_empty()
                || matches!(
                    rv,
                    Rvalue::Ref { .. }
                        | Rvalue::RawRef { .. }
                        | Rvalue::Array(_)
                        | Rvalue::Aggregate(_)
                        | Rvalue::Enum { .. }
                        | Rvalue::Repeat { .. }
                )
        }
        Stmt::Nop => false,
    }) || matches!(
        &b.term,
        Terminator::Call { func: CallTarget::Indirect, .. }
    )
}

/// True when a block contains a call that is not known to be pure.
fn block_has_impure_call(b: &BasicBlock, pure_fns: &HashSet<String>) -> bool {
    match &b.term {
        Terminator::Call { func: CallTarget::Direct(name), .. } => {
            !call_is_pure(name, pure_fns)
        }
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
            // A projected place (`_1[_7].0`) reads its index locals as well; the
            // value it loads is only loop-invariant when every one of them is.
            for proj in &p.proj {
                if let Proj::IndexLocal(i) = proj {
                    v.push(Place { local: *i, proj: Vec::new() });
                }
            }
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
        Rvalue::Array(ops) | Rvalue::Aggregate(ops) | Rvalue::Enum { ops, .. } => {
            ops.iter().for_each(|op| push(op))
        }
        Rvalue::WithOverflow { a, b, .. } => {
            push(a);
            push(b);
        }
        _ => {}
    }
    v
}

