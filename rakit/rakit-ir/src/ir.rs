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
    I128,
    F32,
    F64,
    Bool,
    Ptr,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct StackSlot(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternalTarget {
    User { namespace: u32, index: u32 },
    LibCall(String),
    KnownSymbol(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct External {
    pub target: ExternalTarget,
    pub params: Vec<Type>,
    pub rets: Vec<Type>,
}

#[derive(Clone, Copy, Debug)]
pub struct StackSlotData {
    pub size: u32,
    pub align: u32,
}

/// A global value declaration.  The numeric `base` fields index this table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlobalData {
    VMContext,
    Load { base: u32, offset: i64, ty: Type, flags: u16 },
    IAddImm { base: u32, offset: i64, ty: Type },
    Symbol { target: ExternalTarget, offset: i64, colocated: bool, tls: bool },
    DynScaleTargetConst { vector_type: u32 },
}

/// Dynamic vector type metadata used by dynamic stack slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicTypeData {
    pub base_vector_ty: u32,
    pub dynamic_scale: u32,
}

/// A dynamic stack slot; `kind` is 0 for an explicit dynamic slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicSlotData {
    pub kind: u8,
    pub dyn_ty: u32,
}

/// An argument to a block reached from a `try_call`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryArg {
    Value(Value),
    Ret(u32),
    Exn(u32),
}

impl TryArg {
    pub fn as_value(self) -> Option<Value> {
        match self {
            TryArg::Value(v) => Some(v),
            _ => None,
        }
    }
}

/// A block call with block arguments that may be produced by a `try_call`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TryBlockCall {
    pub block: Block,
    pub args: Vec<TryArg>,
}

/// One entry in an exception table.  Entries are kept in source order because
/// the order of catch clauses is significant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExceptionItem {
    Tag(u32, TryBlockCall),
    Default(TryBlockCall),
    Context(Value),
}

impl Type {
    pub fn is_int(self) -> bool {
        matches!(self, Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::I128 | Type::Ptr)
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
pub enum UnOp {
    Uextend,
    Sextend,
    Ireduce,
    Bint,
    Bnot,
    Bitcast,
    Fpromote,
    Fdemote,
    FcvtFromInt,
    FcvtToInt,
    FcvtFromUint,
    FcvtToUint,
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
    /// Type conversion: result, opcode, result type, operand.
    Unary(Value, UnOp, Type, Value),
    Icmp(Value, IntCc, Type, Value, Value),
    Fcmp(Value, FloatCc, Type, Value, Value),
    Select(Value, Type, Value, Value, Value),
    /// Address of a stack slot plus a constant byte offset.
    StackAddr(Value, StackSlot, i32),
    /// Load a scalar from `addr + offset`.
    Load(Value, Type, Value, i32),
    /// Store a scalar to `addr + offset`.
    Store(Type, Value, i32, Value),
    /// Direct external call: results, external index, arguments.
    Call(Vec<Value>, u32, Vec<Value>),
    /// Indirect call: results, parameter types, return types, callee, arguments.
    CallIndirect {
        results: Vec<Value>,
        params: Vec<Type>,
        rets: Vec<Type>,
        call_conv: u8,
        callee: Value,
        args: Vec<Value>,
    },
    /// Read a declared global value.
    GlobalValue(Value, u32),
    /// Address of a function in the external table.
    FuncAddr(Value, u32),
    /// Address of a dynamic stack slot.
    DynamicStackAddr(Value, u32),
    /// Call with an exception table.  `results` are the normal return values.
    TryCall {
        results: Vec<Value>,
        target: u32,
        args: Vec<Value>,
        normal: TryBlockCall,
        exception: Vec<ExceptionItem>,
    },
    /// Indirect call with an exception table.
    TryCallIndirect {
        results: Vec<Value>,
        params: Vec<Type>,
        rets: Vec<Type>,
        call_conv: u8,
        callee: Value,
        args: Vec<Value>,
        normal: TryBlockCall,
        exception: Vec<ExceptionItem>,
    },
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
            | Inst::Unary(v, _, _, _)
            | Inst::Icmp(v, _, _, _, _)
            | Inst::Fcmp(v, _, _, _, _)
            | Inst::Select(v, _, _, _, _)
            | Inst::StackAddr(v, _, _)
            | Inst::Load(v, _, _, _)
            | Inst::GlobalValue(v, _)
            | Inst::FuncAddr(v, _)
            | Inst::DynamicStackAddr(v, _) => Some(*v),
            Inst::Call(results, _, _)
            | Inst::CallIndirect { results, .. }
            | Inst::TryCall { results, .. }
            | Inst::TryCallIndirect { results, .. } => results.first().copied(),
            Inst::Store(..) | Inst::Jump(..) | Inst::Brif(..) | Inst::Return(..) => None,
        }
    }

    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Inst::Jump(..)
                | Inst::Brif(..)
                | Inst::Return(..)
                | Inst::TryCall { .. }
                | Inst::TryCallIndirect { .. }
        )
    }

    pub fn result_values(&self) -> Vec<Value> {
        match self {
            Inst::Call(results, _, _)
            | Inst::CallIndirect { results, .. }
            | Inst::TryCall { results, .. }
            | Inst::TryCallIndirect { results, .. } => results.clone(),
            _ => self.result_value().into_iter().collect(),
        }
    }

    pub fn operands(&self) -> Vec<Value> {
        match self {
            Inst::Nop(..)
            | Inst::Iconst(..)
            | Inst::F32const(..)
            | Inst::F64const(..)
            | Inst::StackAddr(..) => Vec::new(),
            Inst::Copy(_, _, x) => vec![*x],
            Inst::Bin(_, _, _, x, y) => vec![*x, *y],
            Inst::Unary(_, _, _, x) => vec![*x],
            Inst::Icmp(_, _, _, x, y) => vec![*x, *y],
            Inst::Fcmp(_, _, _, x, y) => vec![*x, *y],
            Inst::Select(_, _, c, x, y) => vec![*c, *x, *y],
            Inst::Load(_, _, a, _) => vec![*a],
            Inst::Store(_, a, _, v) => vec![*a, *v],
            Inst::Call(_, _, args) => args.clone(),
            Inst::CallIndirect { callee, args, .. } => {
                let mut v = Vec::with_capacity(args.len() + 1);
                v.push(*callee);
                v.extend_from_slice(args);
                v
            }
            Inst::GlobalValue(..) | Inst::FuncAddr(..) | Inst::DynamicStackAddr(..) => Vec::new(),
            Inst::TryCall { args, normal, exception, .. } => {
                let mut v = args.clone();
                for arg in &normal.args {
                    if let Some(x) = arg.as_value() {
                        v.push(x);
                    }
                }
                for item in exception {
                    match item {
                        ExceptionItem::Tag(_, call) | ExceptionItem::Default(call) => {
                            for arg in &call.args {
                                if let Some(x) = arg.as_value() {
                                    v.push(x);
                                }
                            }
                        }
                        ExceptionItem::Context(x) => v.push(*x),
                    }
                }
                v
            }
            Inst::TryCallIndirect { callee, args, normal, exception, .. } => {
                let mut v = Vec::with_capacity(args.len() + 1);
                v.push(*callee);
                v.extend_from_slice(args);
                for arg in &normal.args {
                    if let Some(x) = arg.as_value() {
                        v.push(x);
                    }
                }
                for item in exception {
                    match item {
                        ExceptionItem::Tag(_, call) | ExceptionItem::Default(call) => {
                            for arg in &call.args {
                                if let Some(x) = arg.as_value() {
                                    v.push(x);
                                }
                            }
                        }
                        ExceptionItem::Context(x) => v.push(*x),
                    }
                }
                v
            }
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

    pub fn replace_value(&mut self, old: Value, new: Value) {
        fn r(v: &mut Value, old: Value, new: Value) {
            if *v == old {
                *v = new;
            }
        }
        fn r_args(args: &mut [Value], old: Value, new: Value) {
            for v in args {
                r(v, old, new);
            }
        }
        fn r_try(args: &mut [TryArg], old: Value, new: Value) {
            for arg in args {
                if let TryArg::Value(v) = arg {
                    r(v, old, new);
                }
            }
        }
        match self {
            Inst::Nop(..)
            | Inst::Iconst(..)
            | Inst::F32const(..)
            | Inst::F64const(..)
            | Inst::StackAddr(..)
            | Inst::GlobalValue(..)
            | Inst::FuncAddr(..)
            | Inst::DynamicStackAddr(..) => {}
            Inst::Copy(_, _, x) => r(x, old, new),
            Inst::Bin(_, _, _, x, y) | Inst::Icmp(_, _, _, x, y) | Inst::Fcmp(_, _, _, x, y) => {
                r(x, old, new);
                r(y, old, new);
            }
            Inst::Unary(_, _, _, x) => r(x, old, new),
            Inst::Select(_, _, c, x, y) => {
                r(c, old, new);
                r(x, old, new);
                r(y, old, new);
            }
            Inst::Load(_, _, a, _) => r(a, old, new),
            Inst::Store(_, a, _, v) => {
                r(a, old, new);
                r(v, old, new);
            }
            Inst::Call(_, _, args) => r_args(args, old, new),
            Inst::CallIndirect { callee, args, .. } => {
                r(callee, old, new);
                r_args(args, old, new);
            }
            Inst::TryCall { args, normal, exception, .. } => {
                r_args(args, old, new);
                r_try(&mut normal.args, old, new);
                for item in exception {
                    match item {
                        ExceptionItem::Tag(_, call) | ExceptionItem::Default(call) => {
                            r_try(&mut call.args, old, new)
                        }
                        ExceptionItem::Context(v) => r(v, old, new),
                    }
                }
            }
            Inst::TryCallIndirect { callee, args, normal, exception, .. } => {
                r(callee, old, new);
                r_args(args, old, new);
                r_try(&mut normal.args, old, new);
                for item in exception {
                    match item {
                        ExceptionItem::Tag(_, call) | ExceptionItem::Default(call) => {
                            r_try(&mut call.args, old, new)
                        }
                        ExceptionItem::Context(v) => r(v, old, new),
                    }
                }
            }
            Inst::Jump(_, args) | Inst::Return(args) => r_args(args, old, new),
            Inst::Brif(c, _, ta, _, fa) => {
                r(c, old, new);
                r_args(ta, old, new);
                r_args(fa, old, new);
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
    /// Fixed-size stack slots addressed by `StackAddr`.
    pub stack_slots: Vec<StackSlotData>,
    /// Dynamic stack slots addressed by `DynamicStackAddr`.
    pub dynamic_stack_slots: Vec<DynamicSlotData>,
    /// Dynamic vector type metadata referenced by dynamic stack slots.
    pub dynamic_types: Vec<DynamicTypeData>,
    /// Global value declarations addressed by `GlobalValue`.
    pub globals: Vec<GlobalData>,
    /// Optional global value used as the stack limit.
    pub stack_limit: Option<u32>,
    /// Direct external functions referenced by `Call`/`FuncAddr`.
    pub externals: Vec<External>,
    /// Target name of this function when it can call itself.
    pub self_target: Option<ExternalTarget>,
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
            stack_slots: Vec::new(),
            dynamic_stack_slots: Vec::new(),
            dynamic_types: Vec::new(),
            globals: Vec::new(),
            stack_limit: None,
            externals: Vec::new(),
            self_target: None,
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
