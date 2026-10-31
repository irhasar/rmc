//! Abstract syntax tree for the Rust subset accepted by nrc.
//!
//! The tree is produced by the parser in an untyped form and then annotated in
//! place by the semantic analyzer. `Expr::Var` carries the resolved stack slot
//! and type; `slot == -1` marks a not-yet-resolved node.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    I64,
    Bool,
    Unit,
    Array(Box<Type>, usize),
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::I64 | Type::Bool => 8,
            Type::Unit => 0,
            Type::Array(elem, n) => elem.size() * n,
        }
    }

    /// Element size for array indexing; only 8-byte elements are supported.
    pub fn is_scalar(&self) -> bool {
        matches!(self, Type::I64 | Type::Bool)
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
    /// [value; count]
    ArrayLit(Box<Expr>, usize),
    /// expr as Type
    Cast(Box<Expr>, Type),
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
    Expr(Expr),
    While(Expr, Block),
    Loop(Block),
    Return(Option<Expr>),
    Break,
    Continue,
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
    pub fns: Vec<FnDef>,
}
