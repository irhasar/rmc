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

/// Returns whether a self-recursive linear recurrence was rewritten.
pub fn run(func: &mut ir::Function) -> bool {
    if std::env::var_os("RAKIT_NO_REC_LOOP").is_some() {
        return false;
    }
    let Some(p) = try_match(func) else { return false };
    rewrite(func, p);
    true
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
    let (call1, fr1, t1, c1, next1) = call_sub(func, rblk, n)?;
    // The two self-calls may use distinct FuncRefs (distinct sigs) that both
    // resolve to this function; require each to be a self-call, not equality.
    // Check the first before scanning the next arm: if `rblk` holds a
    // non-self call the match is dead and scanning `next1` is wasted.
    if !is_self(func, fr1) {
        return None;
    }
    let (call2, fr2, t2, c2, next2) = call_sub(func, next1, n)?;
    if !is_self(func, fr2) {
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
    // `rewrite` deletes every one of these blocks. Anything in them other than
    // the two recursive calls is dropped, so a base-case store or any other
    // observable effect must veto the transform.
    if !deleted_blocks_are_safe(func, &old_blocks, &[call1, call2]) {
        return None;
    }
    Some(Pat { entry, n, base, c1, c2, func_ref: fr1, old_blocks })
}

/// `rewrite` removes every block in `blocks`. Reject the match if one of them
/// does anything observable besides the two recursive calls it replaces: a
/// store, another call, a trapping operation, or a value used outside the
/// removed region.
fn deleted_blocks_are_safe(func: &ir::Function, blocks: &[Block], calls: &[Inst]) -> bool {
    for &block in blocks {
        for inst in func.layout.block_insts(block) {
            if calls.contains(&inst) {
                continue;
            }
            let op = func.dfg.insts[inst].opcode();
            if op == Opcode::Nop || op.is_branch() || op.is_return() || op.is_terminator() {
                continue;
            }
            if op.is_call()
                || op.can_load()
                || op.can_store()
                || op.can_trap()
                || op.other_side_effects()
            {
                return false;
            }
            for &result in func.dfg.inst_results(inst) {
                if result_used_outside(func, result, blocks) {
                    return false;
                }
            }
        }
    }
    true
}

fn result_used_outside(func: &ir::Function, value: Value, blocks: &[Block]) -> bool {
    let value = func.dfg.resolve_aliases(value);
    for block in func.layout.blocks() {
        if blocks.contains(&block) {
            continue;
        }
        for inst in func.layout.block_insts(block) {
            for arg in func.dfg.insts[inst].arguments(&func.dfg.value_lists) {
                if func.dfg.resolve_aliases(*arg) == value {
                    return true;
                }
            }
            for call in func.dfg.insts[inst]
                .branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables)
            {
                for arg in call.args(&func.dfg.value_lists) {
                    if arg.as_value().map(|a| func.dfg.resolve_aliases(a)) == Some(value) {
                        return true;
                    }
                }
            }
        }
    }
    false
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
) -> Option<(Inst, ir::FuncRef, Value, i64, Block)> {
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
        found = Some((inst, func_ref, res, c, next));
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
