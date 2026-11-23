//! An ergonomic builder for Rakit IR functions.

use crate::ir::{
    BinOp, Block, BlockData, FloatCc, Function, Inst, IntCc, StackSlot, StackSlotData, Type, Value,
};

pub struct Builder {
    func: Function,
}

impl Builder {
    pub fn new(ret: Vec<Type>, entry_params: Vec<Type>) -> Self {
        Builder { func: Function::new(ret, entry_params) }
    }

    pub fn func(&self) -> &Function {
        &self.func
    }

    pub fn into_func(self) -> Function {
        self.func
    }

    pub fn add_block(&mut self, params: Vec<Type>) -> Block {
        let b = Block(self.func.blocks.len() as u32);
        let params = params.into_iter().map(|ty| self.new_value(ty)).collect();
        self.func.blocks.push(BlockData { params, insts: Vec::new() });
        b
    }

    fn new_value(&mut self, ty: Type) -> Value {
        let v = Value(self.func.values.len() as u32);
        self.func.values.push(ty);
        v
    }

    fn push(&mut self, b: Block, inst: Inst) {
        self.func.blocks[b.0 as usize].insts.push(inst);
    }

    pub fn iconst(&mut self, b: Block, ty: Type, v: i64) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Iconst(r, ty, v));
        r
    }

    pub fn f32const(&mut self, b: Block, v: f32) -> Value {
        let r = self.new_value(Type::F32);
        self.push(b, Inst::F32const(r, v));
        r
    }

    pub fn f64const(&mut self, b: Block, v: f64) -> Value {
        let r = self.new_value(Type::F64);
        self.push(b, Inst::F64const(r, v));
        r
    }

    pub fn copy(&mut self, b: Block, ty: Type, x: Value) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Copy(r, ty, x));
        r
    }

    pub fn nop(&mut self, b: Block, ty: Type) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Nop(r, ty));
        r
    }

    pub fn bin(&mut self, b: Block, op: BinOp, ty: Type, x: Value, y: Value) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Bin(r, op, ty, x, y));
        r
    }

    pub fn icmp(&mut self, b: Block, cc: IntCc, ty: Type, x: Value, y: Value) -> Value {
        let r = self.new_value(Type::Bool);
        self.push(b, Inst::Icmp(r, cc, ty, x, y));
        r
    }

    pub fn fcmp(&mut self, b: Block, cc: FloatCc, ty: Type, x: Value, y: Value) -> Value {
        let r = self.new_value(Type::Bool);
        self.push(b, Inst::Fcmp(r, cc, ty, x, y));
        r
    }

    pub fn select(&mut self, b: Block, ty: Type, c: Value, x: Value, y: Value) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Select(r, ty, c, x, y));
        r
    }

    pub fn add_stack_slot(&mut self, size: u32, align: u32) -> StackSlot {
        let s = StackSlot(self.func.stack_slots.len() as u32);
        self.func.stack_slots.push(StackSlotData { size, align });
        s
    }

    pub fn stack_addr(&mut self, b: Block, slot: StackSlot, offset: i32) -> Value {
        let r = self.new_value(Type::I64);
        self.push(b, Inst::StackAddr(r, slot, offset));
        r
    }

    pub fn load(&mut self, b: Block, ty: Type, addr: Value, offset: i32) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Load(r, ty, addr, offset));
        r
    }

    pub fn store(&mut self, b: Block, ty: Type, addr: Value, offset: i32, value: Value) {
        self.push(b, Inst::Store(ty, addr, offset, value));
    }

    pub fn jump(&mut self, b: Block, target: Block, args: Vec<Value>) {
        self.push(b, Inst::Jump(target, args));
    }

    pub fn brif(
        &mut self,
        b: Block,
        cond: Value,
        then_block: Block,
        then_args: Vec<Value>,
        else_block: Block,
        else_args: Vec<Value>,
    ) {
        self.push(b, Inst::Brif(cond, then_block, then_args, else_block, else_args));
    }

    pub fn call(
        &mut self,
        b: Block,
        target: u32,
        args: Vec<Value>,
        ret: Option<Type>,
    ) -> Option<Value> {
        let result = ret.map(|ty| self.new_value(ty));
        self.push(b, Inst::Call(result, target, args));
        result
    }

    pub fn ret(&mut self, b: Block, args: Vec<Value>) {
        self.push(b, Inst::Return(args));
    }
}
