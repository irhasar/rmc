//! Convert a self-recursive linear-recurrence function
//!   `F(x) = if x < base { x } else { F(x - c1) + F(x - c2) }`
//! into a loop with a single recursive call per iteration:
//!   `acc = 0; while x >= base { acc += F(x - c1); x -= c2; } return acc + x`
//!
//! LLVM does exactly this in its middle-end (`TailCallElim`), which rakit does
//! not have. RAKIT is too low-level for a general pass, but the `fib`-shaped
//! pattern is cleanly recoverable and halves the number of recursive calls.
//! The rewrite is only applied when every piece matches exactly (self-call,
//! identity base, `+` of the two call results), so it is conservative.
use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::condcodes::IntCC;
use rakit_codegen::ir::{self, Block, Inst, InstBuilder, Opcode, Value};

pub fn run(func: &mut ir::Function) {
    if std::env::var_os("RAKIT_NO_REC_LOOP").is_some() {
        return;
    }
    let Some(p) = try_match(func) else { return };
    rewrite(func, p);
}

struct Pat {
    entry: Block,
    n: Value,
    base: i64,
    c1: i64,
    c2: i64,
    func_ref: ir::FuncRef,
    old_blocks: Vec<Block>,
}

fn try_match(func: &ir::Function) -> Option<Pat> {
    let dfg = &func.dfg;
    let entry = func.layout.entry_block()?;
    let params = dfg.block_params(entry);
    if params.len() != 1 {
        return None;
    }
    let n = params[0];
    if dfg.value_type(n) != ir::types::I64 {
        return None;
    }
    let h = jump_target(func, entry)?;
    let ir::InstructionData::Brif { arg: cond, blocks, .. } =
        dfg.insts[func.layout.last_inst(h)?]
    else {
        return None;
    };
    let (cc, cx, cy) = icmp_parts(dfg, cond)?;
    if cc != IntCC::UnsignedLessThan || dfg.resolve_aliases(cx) != n {
        return None;
    }
    let base = iconst_i64(dfg, dfg.resolve_aliases(cy))?;
    let bblk = blocks[0].block(&dfg.value_lists);
    let rblk = blocks[1].block(&dfg.value_lists);
    // Base block returns the argument unchanged.
    let bargs = ret_args(func, bblk)?;
    if bargs.len() != 1 || dfg.resolve_aliases(bargs[0]) != n {
        return None;
    }
    let (fr1, t1, c1, next1) = call_sub(func, rblk, n)?;
    let (fr2, t2, c2, next2) = call_sub(func, next1, n)?;
    // The two self-calls may use distinct FuncRefs (distinct sigs) that both
    // resolve to this function; require both to be self-calls, not equality.
    if !is_self(func, fr1) || !is_self(func, fr2) {
        return None;
    }
    let rargs = ret_args(func, next2)?;
    if rargs.len() != 1 {
        return None;
    }
    let sum = dfg.resolve_aliases(rargs[0]);
    let ir::ValueDef::Result(si, _) = dfg.value_def(sum) else { return None };
    let ir::InstructionData::Binary { opcode: Opcode::Iadd, args: sargs, .. } = dfg.insts[si]
    else {
        return None;
    };
    let (sa, sb) = (dfg.resolve_aliases(sargs[0]), dfg.resolve_aliases(sargs[1]));
    if !((sa == t1 && sb == t2) || (sa == t2 && sb == t1)) {
        return None;
    }
    let mut old_blocks = vec![h, bblk, rblk, next1, next2];
    old_blocks.sort();
    old_blocks.dedup();
    if old_blocks.len() != 5 {
        return None;
    }
    Some(Pat { entry, n, base, c1, c2, func_ref: fr1, old_blocks })
}

fn rewrite(func: &mut ir::Function, p: Pat) {
    let loop_blk = func.dfg.make_block();
    let base_blk = func.dfg.make_block();
    let rec_blk = func.dfg.make_block();
    func.layout.append_block(loop_blk);
    func.layout.append_block(base_blk);
    func.layout.append_block(rec_blk);

    let n_param = func.dfg.append_block_param(loop_blk, ir::types::I64);
    let acc_param = func.dfg.append_block_param(loop_blk, ir::types::I64);
    let bn = func.dfg.append_block_param(base_blk, ir::types::I64);
    let bacc = func.dfg.append_block_param(base_blk, ir::types::I64);
    let rn = func.dfg.append_block_param(rec_blk, ir::types::I64);
    let racc = func.dfg.append_block_param(rec_blk, ir::types::I64);

    let eterm = func.layout.last_inst(p.entry).unwrap();
    func.layout.remove_inst(eterm);
    {
        let mut cur = FuncCursor::new(func).at_bottom(p.entry);
        let zero = cur.ins().iconst(ir::types::I64, 0);
        cur.ins().jump(loop_blk, &[p.n.into(), zero.into()]);
    }
    {
        let mut cur = FuncCursor::new(func).at_bottom(loop_blk);
        let cbase = cur.ins().iconst(ir::types::I64, p.base);
        let cond = cur.ins().icmp(IntCC::UnsignedLessThan, n_param, cbase);
        cur.ins().brif(
            cond,
            base_blk,
            &[n_param.into(), acc_param.into()],
            rec_blk,
            &[n_param.into(), acc_param.into()],
        );
    }
    {
        let mut cur = FuncCursor::new(func).at_bottom(base_blk);
        let r = cur.ins().iadd(bn, bacc);
        cur.ins().return_(&[r]);
    }
    {
        let mut cur = FuncCursor::new(func).at_bottom(rec_blk);
        let c1 = cur.ins().iconst(ir::types::I64, p.c1);
        let n1 = cur.ins().isub(rn, c1);
        let call = cur.ins().call(p.func_ref, &[n1]);
        let t1 = *cur.func.dfg.inst_results(call).first().unwrap();
        let acc1 = cur.ins().iadd(racc, t1);
        let c2 = cur.ins().iconst(ir::types::I64, p.c2);
        let n2 = cur.ins().isub(rn, c2);
        cur.ins().jump(loop_blk, &[n2.into(), acc1.into()]);
    }

    for b in p.old_blocks {
        func.layout.remove_block_and_insts(b);
    }
}

fn is_self(func: &ir::Function, fr: ir::FuncRef) -> bool {
    let ir::ExternalName::User(reff) = func.dfg.ext_funcs[fr].name else { return false };
    let idx = func.params.user_named_funcs()[reff].index;
    matches!(&func.name, ir::UserFuncName::User(u) if u.index == idx)
}

fn jump_target(func: &ir::Function, block: Block) -> Option<Block> {
    let ir::InstructionData::Jump { destination, .. } =
        func.dfg.insts[func.layout.last_inst(block)?]
    else {
        return None;
    };
    Some(destination.block(&func.dfg.value_lists))
}

/// Arguments of a `return` terminator, if `block` ends in one.
fn ret_args<'a>(func: &'a ir::Function, block: Block) -> Option<&'a [Value]> {
    let inst = func.layout.last_inst(block)?;
    if func.dfg.insts[inst].opcode() != Opcode::Return {
        return None;
    }
    Some(func.dfg.insts[inst].arguments(&func.dfg.value_lists))
}

/// Match a block that holds exactly one `call F(n - c)` and then jumps on.
fn call_sub(
    func: &ir::Function,
    block: Block,
    n: Value,
) -> Option<(ir::FuncRef, Value, i64, Block)> {
    let next = jump_target(func, block)?;
    let mut found = None;
    for inst in func.layout.block_insts(block) {
        let ir::InstructionData::Call { func_ref, .. } = func.dfg.insts[inst] else { continue };
        let args = func.dfg.insts[inst].arguments(&func.dfg.value_lists);
        if args.len() != 1 {
            continue;
        }
        let a = func.dfg.resolve_aliases(args[0]);
        let Some(c) = sub_const(&func.dfg, a, n) else { continue };
        let res = *func.dfg.inst_results(inst).first()?;
        found = Some((func_ref, res, c, next));
    }
    found
}

fn sub_const(dfg: &ir::DataFlowGraph, a: Value, n: Value) -> Option<i64> {
    let ir::ValueDef::Result(inst, 0) = dfg.value_def(a) else { return None };
    let ir::InstructionData::Binary { opcode: Opcode::Isub, args, .. } = dfg.insts[inst] else {
        return None;
    };
    if dfg.resolve_aliases(args[0]) != n {
        return None;
    }
    iconst_i64(dfg, dfg.resolve_aliases(args[1]))
}

fn icmp_parts(dfg: &ir::DataFlowGraph, cond: Value) -> Option<(IntCC, Value, Value)> {
    let cond = dfg.resolve_aliases(cond);
    let ir::ValueDef::Result(inst, 0) = dfg.value_def(cond) else { return None };
    let ir::InstructionData::IntCompare { cond: cc, args, .. } = dfg.insts[inst] else {
        return None;
    };
    Some((cc, args[0], args[1]))
}

fn iconst_i64(dfg: &ir::DataFlowGraph, v: Value) -> Option<i64> {
    let ir::ValueDef::Result(inst, 0) = dfg.value_def(v) else { return None };
    let ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. } = dfg.insts[inst] else {
        return None;
    };
    Some(imm.bits())
}
