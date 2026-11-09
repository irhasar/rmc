//! Abstract syntax tree for the Rust subset accepted by rmc.
//!
//! The tree is produced by the parser in an untyped form and then annotated in
//! place by the semantic analyzer. `Expr::Var` carries the resolved stack slot
//! and type; `slot == -1` marks a not-yet-resolved node. `Type::Named` is a
//! placeholder for a struct name that the parser could not resolve; sema
//! rewrites every one of them into `Type::Struct` or a builtin.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    I64,
    Bool,
    Unit,
    Array(Box<Type>, usize),
    /// A struct: its name and its size in bytes. Sizes are computed by sema so
    /// that `size` needs no lookup table.
    Struct(String, usize),
    /// A unit enum, represented by its `i64` tag. It occupies one slot like an
    /// integer, so codegen emits it exactly as it emits `i64`.
    Enum(String),
    /// An enum with at least one payload variant: a tagged union laid out as
    /// `{ i64 tag, <max payload bytes> }`. Its size is computed by sema and
    /// carried here, so codegen needs no lookup. It is an aggregate: named by
    /// its address, passed and returned by pointer, like a struct.
    EnumData(String, usize),
    /// A reference: `&T` or `&mut T`. It occupies one 8-byte slot holding the
    /// address of a `T`, so it is a scalar (passed and returned in one register)
    /// even when `T` is an aggregate. There is no lifetime or borrow checking:
    /// rmc trusts the programmer not to outlive the referent.
    Ref(Box<Type>, bool),
    /// A string literal: a pointer to a NUL-terminated byte string in static
    /// data. It occupies one slot and travels in one register like `i64`, but it
    /// is a distinct type so it cannot be added to an integer or passed where an
    /// `i64` is expected (the ways rmc used to let a bad string through). There
    /// is no way to name this type in source, so only a string literal produces
    /// it.
    Str,
    /// Unresolved name produced by the parser; never survives sema.
    Named(String),
    /// The type of an expression that never produces a value (`!`). Only ever
    /// produced by sema (e.g. a `loop` with no reachable `break`), never named
    /// in source. It unifies with every other type.
    Never,
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::I64 | Type::Bool | Type::Str | Type::Enum(_) | Type::Ref(..) => 8,
            Type::EnumData(_, size) => *size,
            Type::Unit | Type::Never => 0,
            Type::Array(elem, n) => elem.size() * n,
            Type::Struct(_, s) => *s,
            Type::Named(_) => 0,
        }
    }

    /// True for the types that fit in a single 8-byte slot and travel in one
    /// register: `i64`, `bool` and an enum. They may be parameters, fields and
    /// return values.
    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool | Type::Str | Type::Enum(_) | Type::Ref(..))
    }

    /// True for the types an array element may have: the 8-byte integer-like
    /// scalars `i64`, `bool` and a unit enum. A unit enum occupies one slot and
    /// codegen emits it exactly as it emits `i64`, so `[E; N]` lays out and
    /// indexes correctly. A payload enum or a reference is excluded, matching
    /// rustc, which rejects both as array elements.
    pub fn is_int_or_bool(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool | Type::Enum(_))
    }

    /// True for the types an array element may have now that nested aggregates
    /// are supported: the integer-like scalars plus a nested array or a struct.
    /// Every such type is a multiple of 8 bytes, so an element's stride is
    /// `size() / 8` slots. A payload enum, a reference and `str` stay excluded
    /// (rustc rejects a payload enum and a reference as array elements too).
    pub fn is_array_elem(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool | Type::Enum(_) | Type::Array(..) | Type::Struct(..))
    }

    /// True for the aggregate types that live in memory: arrays and structs.
    /// They are named by their address, so they are passed to and returned from
    /// a function by pointer rather than by value.
    pub fn is_aggregate(&self) -> bool {
        matches!(self, Type::Array(..) | Type::Struct(..) | Type::EnumData(..))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Bool(bool),
    /// String literal; evaluates to a pointer to a `{len, bytes}` record in
    /// `.rodata`. Given type `i64` so it can be passed to runtime helpers.
    Str(String),
    Var {
        name: String,
        slot: i32,
        ty: Type,
    },
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
    /// A call. `arg_slots` holds, per argument, the frame slot and slot count of
    /// the hidden copy used to pass an aggregate by value (`(-1, 0)` for a
    /// scalar). `ret_slot` is the frame slot of the hidden return buffer when the
    /// result is an aggregate (`-1` otherwise). All three are filled in by sema.
    Call {
        name: String,
        args: Vec<Expr>,
        arg_slots: Vec<(i32, usize)>,
        ret_slot: i32,
        ret_ty: Type,
    },
    If(Box<Expr>, Block, Option<Block>),
    /// `loop { .. }`. It is always an expression: a `loop` in statement
    /// position is a `Stmt::Expr` around this node with `slot == -2`, so its
    /// value is discarded and it is typed `()`. Otherwise sema types it as the
    /// type of its `break`s, or `!` when no `break` can be reached, and records
    /// that type here. `slot` is the hidden frame slot holding the loop's value
    /// (`-1` when it yields none, `-2` while untyped, meaning "used as a
    /// statement"). `ty` is `Unit` until sema fills it in; codegen needs it to
    /// report the loop's type to its consumer. The last field is the loop's
    /// label (`'a: loop { .. }` -> `Some("a")`), which a `break 'a` /
    /// `continue 'a` targets; `None` for an unlabelled loop.
    Loop(Block, i32, Type, Option<String>),
    /// Marks a desugared `match`, wrapping the *whole* `if` chain exactly once
    /// (not each arm). A `match` desugars to an `if` chain, and the chain must
    /// be recognised as a unit when deciding whether it is exhaustive. Without
    /// this marker a wildcard arm whose body is itself a `match` on the same
    /// enum value would fuse with the outer chain: `match a { A => 1, _ =>
    /// match a { B => 2 } }` and `match a { A => 1, B => 2 }` desugar to the
    /// same `if` chain, but only the second is exhaustive. The marker, inserted
    /// by `desugar_match` after the whole chain is folded, keeps them apart; it
    /// is transparent to codegen. The `bool` records whether some arm is a
    /// catch-all (`_` or a binding), which makes the match exhaustive
    /// regardless of the variants it names.
    MatchBoundary(Block, bool),
    Block(Block),
    /// array[index]
    Index(Box<Expr>, Box<Expr>),
    /// struct field access `base.name`. `offset` is the byte offset from the base
    /// address and `ty` the field's type; both are filled in by sema.
    ///
    /// `pat_struct` names the struct a destructuring pattern bound this field
    /// through (`S { f }`), when the field was produced by that pattern rather
    /// than by an ordinary `base.name`. Sema checks the base is that struct, so
    /// `let P { x } = q` where `q: Q` is rejected as rustc rejects it; for an
    /// ordinary field access it is `None` and the field is resolved by name.
    Field {
        base: Box<Expr>,
        name: String,
        offset: i32,
        ty: Type,
        pat_struct: Option<String>,
    },
    /// `Name { field: value, ... }`. Sema reorders `fields` into declaration
    /// order, fills in each field's offset and type, and allocates `slot`, a
    /// frame buffer the literal is materialised into so it can be used wherever
    /// a struct value is expected (return, argument, tail expression).
    StructLit { name: String, fields: Vec<FieldInit>, slot: i32 },
    /// `[value; count]`
    ArrayLit(Box<Expr>, usize),
    /// `[a, b, c]`. Sema infers the element type from the values and rewrites
    /// this into a block that builds a local array; the parser cannot, because
    /// it does not type-check.
    ArrayList(Vec<Expr>),
    /// `recv.name(args)`. Sema resolves `name` against `recv`'s struct type and
    /// rewrites this into a `Call` to the mangled method with `recv` prepended,
    /// so codegen never sees a method call.
    MethodCall { recv: Box<Expr>, name: String, args: Vec<Expr> },
    /// `Enum::Variant`. Sema fills `tag` with the variant's index and gives the
    /// expression the enum's type; codegen emits the tag as an `i64`. A unit
    /// enum is exactly this: a named integer with checked constructors, which
    /// makes it a zero-cost `switch`-style state value.
    EnumVariant { enum_name: String, variant: String, tag: i64 },
    /// `Enum::Variant(a, b)`: build a payload variant. Sema checks the arguments
    /// against the variant's fields, records each field's byte offset within the
    /// payload, and allocates `slot` for the value, which codegen fills and
    /// whose address it yields, exactly like a struct literal.
    EnumCtor {
        enum_name: String,
        variant: String,
        tag: i64,
        args: Vec<Expr>,
        arg_offsets: Vec<i32>,
        slot: i32,
        size: usize,
    },
    /// `value == Enum::Variant`, as a `bool`. The single condition a `match` arm
    /// tests. Sema checks the value's enum and the variant, and fills `tag`;
    /// codegen compares the value's tag word. Keeping this one node means the
    /// parser needs no knowledge of whether the enum is a unit enum or a tagged
    /// union, and codegen handles both in one place.
    VariantTest {
        scrutinee: Box<Expr>,
        enum_name: String,
        variant: String,
        tag: i64,
        /// How many payload fields the pattern bound with an explicit argument
        /// pattern (`E::V(a, b)`, a `_`, a nested pattern or a trailing `..`);
        /// -1 when the pattern had no parentheses, which is an error for a
        /// variant that has a payload.
        n_binds: i32,
        /// The pattern ended in `..` (`E::V(a, ..)`), so the remaining payload
        /// fields are ignored rather than named. Only meaningful when the
        /// pattern has an argument list (`n_binds >= 0`).
        has_rest: bool,
    },
    /// `value == true`/`false`, as a `bool`. The arm test a `bool` literal
    /// pattern desugars to. It is a distinct node rather than a plain
    /// `Bin(Eq, ..)` so `if_chain_is_exhaustive` can tell a `match` over a
    /// `bool` (whose `true` and `false` arms together are exhaustive) from a
    /// hand-written `if b == true { .. } else if b == false { .. }`, which rustc
    /// does *not* treat as exhaustive in value position. `desugar_match` only
    /// ever produces it over a `Var` — the scrutinee it bound to a temporary or
    /// a bool variable — so no side effect is evaluated here.
    BoolTest { scrutinee: Box<Expr>, value: bool },
    /// The payload of a payload enum, read after a tag check: field `idx` of
    /// `variant`, at `offset` bytes into the value. Yields the field value, or
    /// its address when the field is itself an aggregate.
    EnumPayload {
        recv: Box<Expr>,
        enum_name: String,
        variant: String,
        idx: usize,
        offset: i32,
        ty: Type,
    },
    /// expr as Type
    Cast(Box<Expr>, Type),
    /// `&e` or `&mut e`: the address of an lvalue, or of a temporary
    /// materialised into `slot` when `e` has no address of its own (a call
    /// result, a literal, an aggregate-valued expression). `slot` is -1 when the
    /// operand is an addressable lvalue. `ty` is the referent type.
    AddrOf { expr: Box<Expr>, ty: Type, mutable: bool, slot: i32 },
    /// `*e`: read the referent. `ty` is the referent type, filled in by sema.
    /// Reading an aggregate yields its address, exactly as naming a struct local
    /// does.
    Deref { expr: Box<Expr>, ty: Type },
}

/// A field inside a struct literal, once sema has resolved its layout.
#[derive(Debug, Clone)]
pub struct FieldInit {
    pub name: String,
    pub offset: i32,
    pub ty: Type,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        ty: Option<Type>,
        init: Option<Expr>,
        slot: i32,
        /// Whether the binding was declared `mut`. Stored so a later assignment
        /// to it can be rejected when it was not.
        mutable: bool,
    },
    Assign {
        target: Expr,
        value: Expr,
    },
    /// An expression evaluated for its side effects.
    Expr(Expr),
    /// `while cond { body }`. The last field is the loop label
    /// (`'a: while c { .. }` -> `Some("a")`), or `None` when unlabelled.
    While(Expr, Block, Option<String>),
    /// `for var in start..end { body }`. A `for` cannot be desugared to a
    /// `while` because `continue` must still run the increment, so it keeps its
    /// own node and the code generator gives the increment its own label.
    For {
        var: String,
        var_slot: i32,
        start: Expr,
        end: Expr,
        /// Hidden slot holding the end bound, evaluated once before the loop.
        end_slot: i32,
        /// `start..=end` includes `end`; `start..end` does not.
        inclusive: bool,
        body: Block,
        /// The loop label (`'a: for .. { .. }` -> `Some("a")`), else `None`.
        label: Option<String>,
    },
    Return(Option<Expr>),
    /// `break`, `break value`, or `break 'label [value]`. A value is only legal
    /// in a `loop`; sema rejects it in a `while`/`for` (rustc E0571). The second
    /// field names the target loop when the `break` is labelled.
    Break(Option<Expr>, Option<String>),
    /// `continue` or `continue 'label`.
    Continue(Option<String>),
}

/// A user-defined struct. Field offsets are assigned by sema in declaration
/// order, so the layout is exactly the declaration order with no padding.
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    /// `Some(T)` when this is a method defined in `impl T { .. }`. The method
    /// takes a leading `self` of type `T`; `self` is not listed in `params`.
    pub self_ty: Option<String>,
    /// True when the first parameter is a method receiver. The parameter is
    /// already present in `params`; this only records that a call to it must go
    /// through a method call, so a free function cannot be called as one.
    pub takes_self: bool,
    /// True when the receiver was written `&self` / `&mut self`. The receiver is
    /// then a reference: a method call passes the value's address instead of a
    /// copy, so a large receiver is not copied into the callee.
    pub ref_self: bool,
    /// True when the receiver was written `&mut self`. It only affects type
    /// checking (`&mut T` and `&T` are distinct types); there is no exclusivity
    /// check.
    pub ref_self_mut: bool,
    pub params: Vec<(String, Type)>,
    /// Whether each parameter was declared `mut`, parallel to `params`. A
    /// non-`mut` parameter is an immutable binding, so assigning it is rejected
    /// like rustc; `mut x: T` allows the body to rebind it (a local copy).
    pub param_mut: Vec<bool>,
    /// Per-parameter destructuring patterns, parallel to `params`. A `None`
    /// entry is a plain `p: T` parameter. A `Some(pattern)` entry is an
    /// aggregate parameter written as a pattern (`S { x, y }: S`, `[a, b]:
    /// [i64; 2]`): the parameter itself is an ordinary named parameter, and
    /// these statements (spliced at the top of the body by `sema`) bind the
    /// pattern's names from it. The pattern has no separate type, so its names
    /// read through the parameter.
    pub param_pats: Vec<Option<Vec<Stmt>>>,
    pub ret: Type,
    pub body: Block,
    /// Frame slot of each parameter, in order, filled in by `sema`. An aggregate
    /// parameter owns `size/8` consecutive slots; a scalar owns one.
    pub param_slots: Vec<i32>,
    /// Frame slot of the hidden pointer buffer for an aggregate return value,
    /// filled in by `sema` (`-1` when the return type is scalar or unit).
    pub ret_slot: i32,
    /// Number of 8-byte stack slots the function needs, filled in by `sema`.
    pub nslots: i32,
}

/// One variant of an enum: a name, its payload field types, and an optional
/// explicit discriminant. An empty `fields` is a unit variant.
#[derive(Debug, Clone)]
pub struct EnumVariantDef {
    pub name: String,
    pub fields: Vec<Type>,
    /// An explicit `= N` discriminant, if written. `None` means the variant
    /// takes the previous variant's value plus one (starting at 0).
    pub discr: Option<i64>,
}

/// A user-defined enum. When every variant is a unit variant the enum is
/// represented by its `i64` tag; when any variant carries a payload it is a
/// tagged union (see `Type::EnumData`).
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<EnumVariantDef>,
}

/// A `const NAME: TYPE = EXPR;` item. rmc resolves it at compile time to a
/// single value and substitutes that value at every use, so a constant has no
/// storage and no codegen. Only `i64` and `bool` constants are accepted, which
/// is what an array length or a scalar expression can use.
#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: String,
    pub ty: Type,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub consts: Vec<ConstDef>,
    pub fns: Vec<FnDef>,
}
