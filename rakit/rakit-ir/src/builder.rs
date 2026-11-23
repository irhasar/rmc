//! An ergonomic builder for Rakit IR functions.

use crate::ir::{
    BinOp, Block, BlockData, ExceptionItem, FloatCc, Function, Inst, IntCc, MemFlags, StackSlot,
    StackSlotData, TryBlockCall, Type, UnOp, Value,
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

    pub fn unary(&mut self, b: Block, op: UnOp, ty: Type, x: Value) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Unary(r, op, ty, x));
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
        self.load_with_flags(b, ty, addr, offset, MemFlags::default())
    }

    pub fn load_with_flags(
        &mut self,
        b: Block,
        ty: Type,
        addr: Value,
        offset: i32,
        flags: MemFlags,
    ) -> Value {
        let r = self.new_value(ty);
        self.push(b, Inst::Load(r, ty, addr, offset, flags));
        r
    }

    pub fn store(&mut self, b: Block, ty: Type, addr: Value, offset: i32, value: Value) {
        self.store_with_flags(b, ty, addr, offset, value, MemFlags::default());
    }

    pub fn store_with_flags(
        &mut self,
        b: Block,
        ty: Type,
        addr: Value,
        offset: i32,
        value: Value,
        flags: MemFlags,
    ) {
        self.push(b, Inst::Store(ty, addr, offset, value, flags));
    }

    pub fn global_value(&mut self, b: Block, index: u32) -> Value {
        let r = self.new_value(Type::I64);
        self.push(b, Inst::GlobalValue(r, index));
        r
    }

    pub fn func_addr(&mut self, b: Block, target: u32) -> Value {
        let r = self.new_value(Type::I64);
        self.push(b, Inst::FuncAddr(r, target));
        r
    }

    pub fn dynamic_stack_addr(&mut self, b: Block, slot: u32) -> Value {
        let r = self.new_value(Type::I64);
        self.push(b, Inst::DynamicStackAddr(r, slot));
        r
    }

    pub fn try_call(
        &mut self,
        b: Block,
        target: u32,
        args: Vec<Value>,
        _rets: Vec<Type>,
        normal: TryBlockCall,
        exception: Vec<ExceptionItem>,
    ) -> Vec<Value> {
        // A `try_call` has no direct SSA results: its normal-return values are
        // passed to `normal` as `TryArg::Ret`.
        self.push(
            b,
            Inst::TryCall { results: Vec::new(), target, args, normal, exception },
        );
        Vec::new()
    }

    pub fn try_call_indirect(
        &mut self,
        b: Block,
        params: Vec<Type>,
        rets: Vec<Type>,
        call_conv: u8,
        callee: Value,
        args: Vec<Value>,
        normal: TryBlockCall,
        exception: Vec<ExceptionItem>,
    ) -> Vec<Value> {
        self.push(
            b,
            Inst::TryCallIndirect {
                results: Vec::new(),
                params,
                rets,
                call_conv,
                callee,
                args,
                normal,
                exception,
            },
        );
        Vec::new()
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
        let rets = ret.into_iter().collect();
        self.call_multi(b, target, args, rets).into_iter().next()
    }

    pub fn call_multi(
        &mut self,
        b: Block,
        target: u32,
        args: Vec<Value>,
        rets: Vec<Type>,
    ) -> Vec<Value> {
        let results: Vec<Value> = rets.into_iter().map(|ty| self.new_value(ty)).collect();
        self.push(b, Inst::Call(results.clone(), target, args));
        results
    }

    pub fn call_indirect(
        &mut self,
        b: Block,
        params: Vec<Type>,
        rets: Vec<Type>,
        call_conv: u8,
        callee: Value,
        args: Vec<Value>,
    ) -> Vec<Value> {
        let results: Vec<Value> = rets.iter().copied().map(|ty| self.new_value(ty)).collect();
        self.push(
            b,
            Inst::CallIndirect { results: results.clone(), params, rets, call_conv, callee, args },
        );
        results
    }

    pub fn ret(&mut self, b: Block, args: Vec<Value>) {
        self.push(b, Inst::Return(args));
    }

    pub fn trap(&mut self, b: Block, code: u8) {
        self.push(b, Inst::Trap(code));
    }
}
