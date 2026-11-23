//! The Rakit mid-level IR: types, values, blocks, instructions and functions.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Value(pub u32);

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub u32);

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b{}", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Ptr,
}

impl Type {
    pub fn is_int(self) -> bool {
        matches!(self, Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Ptr)
    }

    pub fn is_float(self) -> bool {
        matches!(self, Type::F32 | Type::F64)
    }

    pub fn is_bool(self) -> bool {
        matches!(self, Type::Bool)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Sdiv,
    Udiv,
    Srem,
    Urem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Ushr,
    Fadd,
    Fsub,
    Fmul,
    Fdiv,
}

impl BinOp {
    pub fn is_float(self) -> bool {
        matches!(self, BinOp::Fadd | BinOp::Fsub | BinOp::Fmul | BinOp::Fdiv)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IntCc {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FloatCc {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Debug)]
pub enum Inst {
    /// A dead instruction kept as a placeholder so value indices stay stable.
    Nop(Value, Type),
    /// Integer constant of the given type.
    Iconst(Value, Type, i64),
    F32const(Value, f32),
    F64const(Value, f64),
    /// Copy of another value (used by folding to avoid renumbering values).
    Copy(Value, Type, Value),
    Bin(Value, BinOp, Type, Value, Value),
    Icmp(Value, IntCc, Type, Value, Value),
    Fcmp(Value, FloatCc, Type, Value, Value),
    Select(Value, Type, Value, Value, Value),
    Jump(Block, Vec<Value>),
    Brif(Value, Block, Vec<Value>, Block, Vec<Value>),
    Return(Vec<Value>),
}

impl Inst {
    pub fn result_value(&self) -> Option<Value> {
        match self {
            Inst::Nop(v, _)
            | Inst::Iconst(v, _, _)
            | Inst::F32const(v, _)
            | Inst::F64const(v, _)
            | Inst::Copy(v, _, _)
            | Inst::Bin(v, _, _, _, _)
            | Inst::Icmp(v, _, _, _, _)
            | Inst::Fcmp(v, _, _, _, _)
            | Inst::Select(v, _, _, _, _) => Some(*v),
            Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..) => None,
        }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(self, Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..))
    }

    pub fn operands(&self) -> Vec<Value> {
        match self {
            Inst::Nop(..) | Inst::Iconst(..) | Inst::F32const(..) | Inst::F64const(..) => Vec::new(),
            Inst::Copy(_, _, x) => vec![*x],
            Inst::Bin(_, _, _, x, y) => vec![*x, *y],
            Inst::Icmp(_, _, _, x, y) => vec![*x, *y],
            Inst::Fcmp(_, _, _, x, y) => vec![*x, *y],
            Inst::Select(_, _, c, x, y) => vec![*c, *x, *y],
            Inst::Jump(_, args) | Inst::Return(args) => args.clone(),
            Inst::Brif(cond, _, targs, _, fargs) => {
                let mut v = Vec::with_capacity(targs.len() + fargs.len() + 1);
                v.push(*cond);
                v.extend_from_slice(targs);
                v.extend_from_slice(fargs);
                v
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BlockData {
    /// SSA values defined as block parameters (phi nodes).
    pub params: Vec<Value>,
    pub insts: Vec<Inst>,
}

impl BlockData {
    pub fn terminator(&self) -> Option<&Inst> {
        self.insts.last().filter(|i| i.is_terminator())
    }
}

#[derive(Clone, Debug)]
pub struct Function {
    pub blocks: Vec<BlockData>,
    /// Type of every SSA value, indexed by `Value`.
    pub values: Vec<Type>,
    /// Types of the function's return values.
    pub ret: Vec<Type>,
    pub entry: Block,
}

impl Function {
    pub fn new(ret: Vec<Type>, entry_params: Vec<Type>) -> Self {
        let mut values = Vec::new();
        let params = entry_params
            .into_iter()
            .map(|ty| {
                let v = Value(values.len() as u32);
                values.push(ty);
                v
            })
            .collect();
        Function {
            blocks: vec![BlockData { params, insts: Vec::new() }],
            values,
            ret,
            entry: Block(0),
        }
    }

    pub fn block(&self, b: Block) -> &BlockData {
        &self.blocks[b.0 as usize]
    }

    pub fn block_mut(&mut self, b: Block) -> &mut BlockData {
        &mut self.blocks[b.0 as usize]
    }

    pub fn ty(&self, v: Value) -> Type {
        self.values[v.0 as usize]
    }

    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }
}
