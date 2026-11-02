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
    /// Unresolved name produced by the parser; never survives sema.
    Named(String),
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::I64 | Type::Bool => 8,
            Type::Unit => 0,
            Type::Array(elem, n) => elem.size() * n,
            Type::Struct(_, s) => *s,
            Type::Named(_) => 0,
        }
    }

    /// Element size for array indexing; only 8-byte elements are supported.
    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool)
    }

    /// True for the aggregate types that live in memory and cannot be promoted
    /// into a register or passed to a function.
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
    Call(String, Vec<Expr>),
    If(Box<Expr>, Block, Option<Block>),
    Block(Block),
    /// array[index]
    Index(Box<Expr>, Box<Expr>),
    /// struct field access `base.name`. `offset` is the byte offset from the base
    /// address and `ty` the field's type; both are filled in by sema.
    Field { base: Box<Expr>, name: String, offset: i32, ty: Type },
    /// `Name { field: value, ... }`. Sema reorders `fields` into declaration
    /// order and fills in each field's offset and type, so codegen can store
    /// them sequentially without a lookup.
    StructLit { name: String, fields: Vec<FieldInit> },
    /// `[value; count]`
    ArrayLit(Box<Expr>, usize),
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
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub body: Block,
    /// Number of 8-byte stack slots the function needs, filled in by `sema`.
    pub nslots: i32,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub structs: Vec<StructDef>,
    pub fns: Vec<FnDef>,
}
