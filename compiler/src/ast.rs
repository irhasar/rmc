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
    /// Unresolved name produced by the parser; never survives sema.
    Named(String),
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::I64 | Type::Bool | Type::Enum(_) => 8,
            Type::Unit => 0,
            Type::Array(elem, n) => elem.size() * n,
            Type::Struct(_, s) => *s,
            Type::Named(_) => 0,
        }
    }

    /// True for the types that fit in a single 8-byte slot and travel in one
    /// register: `i64`, `bool` and an enum. They may be parameters, fields and
    /// return values.
    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool | Type::Enum(_))
    }

    /// True for the primitive types an array element may have. An enum is
    /// deliberately excluded: codegen only lays out 8-byte integer-like
    /// elements, and `[E; N]` would silently mis-size.
    pub fn is_int_or_bool(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool)
    }

    /// True for the aggregate types that live in memory: arrays and structs.
    /// They are named by their address, so they are passed to and returned from
    /// a function by pointer rather than by value.
    pub fn is_aggregate(&self) -> bool {
        matches!(self, Type::Array(..) | Type::Struct(..))
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
    Block(Block),
    /// array[index]
    Index(Box<Expr>, Box<Expr>),
    /// struct field access `base.name`. `offset` is the byte offset from the base
    /// address and `ty` the field's type; both are filled in by sema.
    Field { base: Box<Expr>, name: String, offset: i32, ty: Type },
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
    /// expr as Type
    Cast(Box<Expr>, Type),
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
    },
    Assign {
        target: Expr,
        value: Expr,
    },
    /// An expression evaluated for its side effects.
    Expr(Expr),
    While(Expr, Block),
    Loop(Block),
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
        body: Block,
    },
    Return(Option<Expr>),
    Break,
    Continue,
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
    pub params: Vec<(String, Type)>,
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

/// A user-defined enum. Every variant is a unit variant (no payload), so the
/// enum is represented by its `i64` tag and codegen needs no new case for it.
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub fns: Vec<FnDef>,
}
