//! If-conversion of two-way diamonds on CLIF.
//!
//! cg_clif lowers a short `if cond { a } else { b }` whose arms only compute a
//! value into a diamond: a `brif` from the head to a *then* and an *else* block,
//! both of which `jump` to a common *join* that receives the chosen value as a
//! parameter. When the arms are small and side-effect-free, the diamond is
//! equivalent to a `select` in the head, which Cranelift lowers to a `cmove`.
//!
//! Two effects matter here. The obvious one is that a `cmove` cannot mispredict,
//! where the original branch can: a data-dependent condition inside a loop (a
//! `hash % k == 0` test) is taken unpredictably and costs a frontend flush every
//! iteration. The second is structural: the fork's `unroll` pass only widens a
//! loop whose body is a single block reached by a single back edge, and a
//! diamond adds blocks, so such a loop is left alone. Converting the diamond
//! first turns the body into the single block the unroller needs.
//!
//! The rewrite is deliberately conservative: both arms must be single-
//! predecessor, single-successor, side-effect-free blocks that end in a `jump`
//! to the same join, carry no block parameters, and define no value that escapes
//! them.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::ir::{self, Block, Inst, InstBuilder, InstructionData, Opcode, Value};
use rustc_data_structures::fx::{FxHashMap, FxHashSet};

/// Cap on the number of instructions an arm may contain. Short arms are the
/// ones that pay off as a `cmove`; a long arm would rather keep its branch.
const MAX_ARM_INSTS: usize = 8;

/// Whether `inst` is safe to evaluate unconditionally: one result, no side
/// effects, and no trap. This mirrors Cranelift's `is_pure_for_egraph`, which
/// the vendored copy does not export.
fn is_speculatable(func: &ir::Function, inst: Inst) -> bool {
    let data = &func.dfg.insts[inst];
    let op = data.opcode();
    let is_pure_load = match data {
        InstructionData::Load { opcode: Opcode::Load, flags, .. } => {
            let flags = func.dfg.mem_flags[*flags];
            flags.readonly() && flags.notrap() && flags.can_move()
        }
        _ => false,
    };
    let trivially_side_effecting = op.is_call()
        || op.is_branch()
        || op.is_terminator()
        || op.is_return()
        || op.can_trap()
        || op.other_side_effects()
        || op.can_store();
    func.dfg.inst_results(inst).len() == 1
        && (is_pure_load || (!op.can_load() && !trivially_side_effecting))
}

/// Whether `op` is cheap enough to run speculatively. A `cmove` computes both
/// arms every iteration, so an arm that expands to a branchy or libcall
/// sequence — a saturating float conversion, or a division/modulo by a
/// non-constant — would cost more than the branch it replaces. A single
/// instruction like `sqrt` or a constant-divisor remainder stays cheap.
fn is_cheap_to_speculate(op: Opcode) -> bool {
    !matches!(
        op,
        Opcode::FcvtToUintSat
            | Opcode::FcvtToSintSat
            | Opcode::Udiv
            | Opcode::Sdiv
            | Opcode::Urem
            | Opcode::Srem
            | Opcode::Fdiv
    )
}

/// Whether a value is defined by an instruction inside `blocks`.
fn defined_in(func: &ir::Function, v: Value, blocks: &FxHashSet<Block>) -> bool {
    match func.dfg.value_def(func.dfg.resolve_aliases(v)) {
        ir::ValueDef::Param(b, _) => blocks.contains(&b),
        ir::ValueDef::Result(inst, _) => {
            func.layout.inst_block(inst).is_some_and(|b| blocks.contains(&b))
        }
        ir::ValueDef::Union(..) => false,
    }
}

/// The `jump` that terminates `block`, if the block ends in one.
fn single_jump(func: &ir::Function, block: Block) -> Option<Inst> {
    let term = func.layout.last_inst(block)?;
    match func.dfg.insts[term] {
        InstructionData::Jump { .. } => Some(term),
        _ => None,
    }
}

/// An arm: the value its `jump` passes to the join.
struct Arm {
    block: Block,
    term: Inst,
    value: Value,
}

/// Recognise `arm` as a pure single-predecessor, single-successor block whose
/// `jump` passes exactly one value to `join`.
fn analyze_arm(
    func: &ir::Function,
    cfg: &cranelift_codegen::flowgraph::ControlFlowGraph,
    arm: Block,
    join: Block,
) -> Option<Arm> {
    if cfg.pred_iter(arm).count() != 1 || cfg.succ_iter(arm).count() != 1 {
        return None;
    }
    if func.dfg.num_block_params(arm) != 0 {
        return None;
    }
    let term = single_jump(func, arm)?;
    let InstructionData::Jump { destination, .. } = func.dfg.insts[term] else {
        return None;
    };
    if destination.block(&func.dfg.value_lists) != join {
        return None;
    }

    // Every non-terminator instruction must be safe to run unconditionally, and
    // the arm must stay small.
    let mut count = 0usize;
    for inst in func.layout.block_insts(arm) {
        if inst == term {
            continue;
        }
        count += 1;
        if count > MAX_ARM_INSTS {
            return None;
        }
        if func.dfg.insts[inst].opcode() == Opcode::Nop {
            continue;
        }
        if !is_speculatable(func, inst) {
            return None;
        }
        if !is_cheap_to_speculate(func.dfg.insts[inst].opcode()) {
            return None;
        }
    }

    // The arm must pass exactly one value to the join.
    let args: Vec<ir::BlockArg> = destination.args(&func.dfg.value_lists).collect();
    if args.len() != 1 {
        return None;
    }
    let value = args[0].as_value()?;
    Some(Arm { block: arm, term, value })
}

/// Whether every value defined by `arm` is used only inside the arms. The
/// selected value is passed to the join by the arm's own `jump`, which is inside
/// the arm, so it is covered by the same rule.
fn arm_values_are_dead_outside(
    func: &ir::Function,
    arm: &Arm,
    arm_blocks: &FxHashSet<Block>,
) -> bool {
    let mut defined: Vec<Value> = Vec::new();
    for inst in func.layout.block_insts(arm.block) {
        if inst == arm.term {
            continue;
        }
        defined.extend_from_slice(func.dfg.inst_results(inst));
    }

    for &v in &defined {
        for block in func.layout.blocks() {
            if arm_blocks.contains(&block) {
                continue;
            }
            for inst in func.layout.block_insts(block) {
                for &arg in func.dfg.insts[inst].arguments(&func.dfg.value_lists) {
                    if func.dfg.resolve_aliases(arg) == v {
                        return false;
                    }
                }
                for call in func
                    .dfg
                    .insts[inst]
                    .branch_destination(&func.dfg.jump_tables, &func.dfg.exception_tables)
                {
                    for arg in call.args(&func.dfg.value_lists) {
                        if arg.as_value().map(|a| func.dfg.resolve_aliases(a)) == Some(v) {
                            return false;
                        }
                    }
                }
            }
        }
    }
    true
}

pub(crate) fn run(func: &mut ir::Function) {
    if !crate::tunables::if_convert_enabled() {
        return;
    }
    // Convert to a fixed point: one rewrite can expose another.
    while convert_one(func) {}
}

/// Find and rewrite a single convertible diamond, returning whether it changed
/// the function.
fn convert_one(func: &mut ir::Function) -> bool {
    let cfg = cranelift_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let blocks: Vec<Block> = func.layout.blocks().collect();

    for &head in &blocks {
        let Some(term) = func.layout.last_inst(head) else { continue };
        let InstructionData::Brif { arg: cond, blocks: arms, .. } = func.dfg.insts[term] else {
            continue;
        };
        let then_block = arms[0].block(&func.dfg.value_lists);
        let else_block = arms[1].block(&func.dfg.value_lists);
        if then_block == else_block {
            continue;
        }

        // Both arms must join at the same block.
        let Some(then_term) = single_jump(func, then_block) else { continue };
        let InstructionData::Jump { destination, .. } = func.dfg.insts[then_term] else {
            continue;
        };
        let join = destination.block(&func.dfg.value_lists);
        if join == head || join == then_block || join == else_block {
            continue;
        }

        // The join takes one parameter and is reached only from the two arms.
        if func.dfg.num_block_params(join) != 1 || cfg.pred_iter(join).count() != 2 {
            continue;
        }
        let join_preds: Vec<Block> = cfg.pred_iter(join).map(|e| e.block).collect();
        if !join_preds.contains(&then_block) || !join_preds.contains(&else_block) {
            continue;
        }

        let Some(then_arm) = analyze_arm(func, &cfg, then_block, join) else { continue };
        let Some(else_arm) = analyze_arm(func, &cfg, else_block, join) else { continue };

        // The condition must be available at the head, and no value the arms
        // compute may escape them.
        let arm_blocks: FxHashSet<Block> = [then_block, else_block].into_iter().collect();
        if defined_in(func, cond, &arm_blocks) {
            continue;
        }
        if !arm_values_are_dead_outside(func, &then_arm, &arm_blocks)
            || !arm_values_are_dead_outside(func, &else_arm, &arm_blocks)
        {
            continue;
        }

        rewrite(func, term, cond, &then_arm, &else_arm, join);
        return true;
    }
    false
}

/// Move the arms' pure instructions into the head, replace the `brif` with a
/// `select` feeding a `jump` to the join, and drop the now-empty arms.
fn rewrite(func: &mut ir::Function, brif: Inst, cond: Value, then_arm: &Arm, else_arm: &Arm, join: Block) {
    // Values defined by the arms' instructions. Aliases of these values are
    // common (cg_clif emits `v6 -> v2` chains), and a cloned instruction that
    // reads such an alias must be redirected to the clone too, or it would keep
    // pointing at the original instruction we are about to delete.
    let mut arm_results: FxHashSet<Value> = FxHashSet::default();
    for arm in [then_arm, else_arm] {
        for inst in func.layout.block_insts(arm.block) {
            if inst == arm.term {
                continue;
            }
            arm_results.extend(func.dfg.inst_results(inst).iter().copied());
        }
    }
    let mut alias_target: FxHashMap<Value, Value> = FxHashMap::default();
    for v in func.dfg.values() {
        let r = func.dfg.resolve_aliases(v);
        if arm_results.contains(&r) {
            alias_target.insert(v, r);
        }
    }

    // Clone each arm's body (minus the terminator) in front of the `brif`,
    // remapping the values the earlier clones produced.
    let mut map: FxHashMap<Value, Value> = FxHashMap::default();
    for arm in [then_arm, else_arm] {
        for inst in func.layout.block_insts(arm.block).collect::<Vec<_>>() {
            if inst == arm.term {
                continue;
            }
            let mut data = func.dfg.insts[inst];
            {
                let mut value_lists = std::mem::take(&mut func.dfg.value_lists);
                let mut jump_tables = std::mem::take(&mut func.dfg.jump_tables);
                let mut exception_tables = std::mem::take(&mut func.dfg.exception_tables);
                data.map_values(
                    &mut value_lists,
                    &mut jump_tables,
                    &mut exception_tables,
                    |v| {
                        let r = *alias_target.get(&v).unwrap_or(&v);
                        *map.get(&r).unwrap_or(&v)
                    },
                );
                func.dfg.value_lists = value_lists;
                func.dfg.jump_tables = jump_tables;
                func.dfg.exception_tables = exception_tables;
            }
            let ctrl_typevar = func.dfg.ctrl_typevar(inst);
            let new_inst = func.dfg.make_inst(data);
            func.dfg.make_inst_results(new_inst, ctrl_typevar);
            let old_results = func.dfg.inst_results(inst).to_vec();
            let new_results = func.dfg.inst_results(new_inst).to_vec();
            for (o, n) in old_results.iter().zip(&new_results) {
                map.insert(*o, *n);
            }
            func.layout.insert_inst(new_inst, brif);
        }
    }

    let then_src = *alias_target.get(&then_arm.value).unwrap_or(&then_arm.value);
    let else_src = *alias_target.get(&else_arm.value).unwrap_or(&else_arm.value);
    let then_val = *map.get(&then_src).unwrap_or(&then_arm.value);
    let else_val = *map.get(&else_src).unwrap_or(&else_arm.value);

    // `select cond, then_val, else_val`, placed in front of the branch.
    let select = {
        let mut pos = FuncCursor::new(func);
        pos.goto_inst(brif);
        pos.ins().select(cond, then_val, else_val)
    };

    // The head now jumps straight to the join, passing the select.
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    func.dfg.insts[brif] = InstructionData::Jump {
        opcode: Opcode::Jump,
        destination: ir::BlockCall::new(join, [ir::BlockArg::Value(select)], &mut pool),
    };
    func.dfg.value_lists = pool;

    // Drop the arms; their instructions were cloned into the head.
    for arm in [then_arm.block, else_arm.block] {
        func.layout.remove_block_and_insts(arm);
    }
}
