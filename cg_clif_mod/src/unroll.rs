//! Loop optimisation on CLIF.
//!
//! Cranelift ships no loop transformations, so a canonical counted loop keeps
//! its full carried dependency chain: `acc = acc*3 + i; i += 1` stays a chain of
//! dependent multiply-adds, one iteration deep. The fix that actually helps is
//! *affine reassociation*: when the loop state is a linear function of itself,
//! its `factor`-fold iterate is also linear and can be evaluated with a handful
//! of multiplies, collapsing `factor` dependent steps into one independent group.
//!
//! This pass recognises a canonical counted loop, derives the affine state
//! update from the body, composes it `factor` times, and rewrites the back edge
//! so one header check may advance the whole state by `factor` iterations at
//! once. A guard keeps the arithmetic exact: the wide step is taken only when
//! every one of those `factor` iterations satisfies the loop test, otherwise the
//! original single step runs. The fallback therefore preserves the tail, and the
//! closed form is exact modulo `2^64` because `iadd`/`imul` wrap.
//!
//! On the scalar benchmark this turns a ~15x LLVM deficit into parity, because
//! LLVM's advantage there is exactly this reassociation (plus unrolling).
//!
//! A body-duplicating unroller is also implemented, but it does not shorten the
//! carried chain and measured slower, so it is off unless `CG_CLIF_UNROLL_CLONE`
//! is set.

use cranelift_codegen::cursor::{Cursor, FuncCursor};
use cranelift_codegen::ir::{
    self, condcodes::IntCC, immediates::Imm64, Block, Inst, InstBuilder, Opcode, Value,
};
use rustc_data_structures::fx::FxHashMap;

/// Number of loop iterations folded into one guarded step.
const UNROLL_FACTOR: u64 = 8;

/// Upper bound on the induction step we are willing to scale, so that the
/// arithmetic stays well clear of the type's range.
const MAX_DELTA: u64 = 1 << 40;

fn unroll_factor() -> u64 {
    match std::env::var("CG_CLIF_UNROLL") {
        Ok(v) => v.parse().unwrap_or(UNROLL_FACTOR),
        Err(_) => UNROLL_FACTOR,
    }
}

struct Plan {
    /// Loop header: the block whose parameters carry the loop state.
    header: Block,
    /// Single-block loop body, branching back to `header`.
    body: Block,
    /// `jump` in the body whose destination is `header`.
    back_edge: Inst,
    /// The induction variable as seen at the top of the header.
    iv: Value,
    /// Loop-invariant bound compared against the induction variable.
    bound: Value,
    /// Comparison direction of the header's continue test.
    cc: IntCC,
    /// Whether the induction variable is the left operand of the compare.
    iv_left: bool,
    /// Constant step applied to the induction variable per iteration.
    delta: u64,
}

pub(crate) fn run(func: &mut ir::Function) {
    let factor = unroll_factor();
    if factor < 2 {
        return;
    }
    let Some(plan) = analyze(func) else { return };
    apply(func, &plan, factor);
}

fn successors(func: &ir::Function, block: Block) -> Vec<Block> {
    let dfg = &func.dfg;
    let mut out = Vec::new();
    for inst in func.layout.block_insts(block) {
        for call in dfg.insts[inst].branch_destination(&dfg.jump_tables, &dfg.exception_tables) {
            out.push(call.block(&dfg.value_lists));
        }
    }
    out
}

fn const_of(func: &ir::Function, v: Value) -> Option<u64> {
    let v = func.dfg.resolve_aliases(v);
    let ir::ValueDef::Result(inst, 0) = func.dfg.value_def(v) else { return None };
    match func.dfg.insts[inst] {
        ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. } => Some(imm.bits() as u64),
        _ => None,
    }
}

/// Whether `v` is defined inside one of `blocks` (as a parameter or a result).
fn defined_in(func: &ir::Function, v: Value, blocks: &[Block]) -> bool {
    let dfg = &func.dfg;
    match dfg.value_def(dfg.resolve_aliases(v)) {
        ir::ValueDef::Param(b, _) => blocks.contains(&b),
        ir::ValueDef::Result(inst, _) => {
            func.layout.inst_block(inst).is_some_and(|b| blocks.contains(&b))
        }
        ir::ValueDef::Union(..) => true,
    }
}

fn analyze(func: &ir::Function) -> Option<Plan> {
    let dfg = &func.dfg;
    let layout = &func.layout;

    for body in layout.blocks() {
        let Some(term) = layout.last_inst(body) else { continue };
        let ir::InstructionData::Jump { destination, opcode: _, .. } = dfg.insts[term] else {
            continue;
        };
        let header = destination.block(&dfg.value_lists);
        if header == body {
            continue;
        }

        // The header must be a two-way branch. Its "then" arm is the body; the
        // other arm leaves the loop.
        let Some(hterm) = layout.last_inst(header) else { continue };
        let ir::InstructionData::Brif { arg: cond, blocks, opcode: _ } = dfg.insts[hterm] else {
            continue;
        };
        if blocks[0].block(&dfg.value_lists) != body {
            continue;
        }

        // The loop must have a single back edge, and it must be the body's.
        let loop_blocks = [header, body];
        let back_edges =
            loop_blocks.iter().filter(|&&b| successors(func, b).contains(&header)).count();
        if back_edges != 1 {
            continue;
        }

        // The continue test must be `icmp(cc, iv, bound)` with a single header
        // parameter on one side and a loop-invariant value on the other.
        let cond = dfg.resolve_aliases(cond);
        let ir::ValueDef::Result(icmp_inst, 0) = dfg.value_def(cond) else { continue };
        let ir::InstructionData::IntCompare { cond: cc, args, opcode: _ } = dfg.insts[icmp_inst]
        else {
            continue;
        };
        // Only increasing loops, so the wide step cannot overshoot the bound.
        let iv_left = match cc {
            IntCC::UnsignedLessThan | IntCC::UnsignedLessThanOrEqual => true,
            IntCC::UnsignedGreaterThan | IntCC::UnsignedGreaterThanOrEqual => false,
            _ => continue,
        };
        let (lhs, rhs) = (args[0], args[1]);

        let param_of = |v: Value| match dfg.value_def(dfg.resolve_aliases(v)) {
            ir::ValueDef::Param(b, i) if b == header => Some(i),
            _ => None,
        };
        let (iv_idx, iv, bound) = if iv_left {
            let Some(i) = param_of(lhs) else { continue };
            (i, lhs, rhs)
        } else {
            let Some(i) = param_of(rhs) else { continue };
            (i, rhs, lhs)
        };
        // The bound must be loop-invariant. A value defined inside the loop is
        // only acceptable when it is a constant, which cg_clif emits inline in
        // the header for a literal trip count.
        let loop_blocks = [header, body];
        if const_of(func, bound).is_none() && defined_in(func, bound, &loop_blocks) {
            continue;
        }

        // The induction step is the back edge's argument for the induction
        // parameter: it must be `iv + <small positive constant>`.
        let ir::InstructionData::Jump { destination: back_call, opcode: _, .. } = dfg.insts[term]
        else {
            unreachable!()
        };
        let args: Vec<ir::BlockArg> = back_call.args(&dfg.value_lists).collect();
        if args.len() != dfg.block_params(header).len() {
            continue;
        }
        let ir::BlockArg::Value(step_val) = args[iv_idx] else { continue };
        let step_val = dfg.resolve_aliases(step_val);
        let ir::ValueDef::Result(step_inst, 0) = dfg.value_def(step_val) else { continue };
        let ir::InstructionData::Binary { opcode: Opcode::Iadd, args: sargs, .. } =
            dfg.insts[step_inst]
        else {
            continue;
        };
        let iv = dfg.resolve_aliases(iv);
        let delta = match (const_of(func, sargs[0]), const_of(func, sargs[1])) {
            (None, Some(d)) if dfg.resolve_aliases(sargs[0]) == iv => d,
            (Some(d), None) if dfg.resolve_aliases(sargs[1]) == iv => d,
            _ => continue,
        };
        if delta == 0 || delta > MAX_DELTA {
            continue;
        }

        return Some(Plan { header, body, back_edge: term, iv, bound, cc, iv_left, delta });
    }
    None
}

/// Copy the body's instructions into `dst`, remapping each header parameter to
/// the corresponding `dst` parameter. Instructions that use header *parameters*
/// (rather than header-local temporaries) therefore read the clone's state.
fn clone_body(func: &mut ir::Function, plan: &Plan, dst: Block) {
    let header_params: Vec<Value> = func.dfg.block_params(plan.header).to_vec();
    let dst_params: Vec<Value> = func.dfg.block_params(dst).to_vec();

    // Map header parameters to the clone's parameters, and every alias of a
    // header parameter to the same clone parameter. cg_clif routinely emits
    // `v6 -> v2` aliases, and the body reads the loop state through those, so
    // resolving them here is what makes the clone read its own state.
    let param_map: FxHashMap<Value, Value> =
        header_params.iter().copied().zip(dst_params.iter().copied()).collect();
    let mut vmap: FxHashMap<Value, Value> = FxHashMap::default();
    for v in func.dfg.values() {
        let resolved = func.dfg.resolve_aliases(v);
        if let Some(&d) = param_map.get(&resolved) {
            vmap.insert(v, d);
        }
    }

    let insts: Vec<Inst> = func.layout.block_insts(plan.body).collect();
    for &old_inst in &insts {
        let mut data = func.dfg.insts[old_inst];
        {
            let mut value_lists = std::mem::take(&mut func.dfg.value_lists);
            let mut jump_tables = std::mem::take(&mut func.dfg.jump_tables);
            let mut exception_tables = std::mem::take(&mut func.dfg.exception_tables);
            // `InstructionData` is `Copy` and its `BlockCall`s share the value
            // pool with the original instruction. Deep-clone them first, or
            // remapping the clone's branch arguments would corrupt the original.
            for call in data.branch_destination_mut(&mut jump_tables, &mut exception_tables) {
                *call = call.deep_clone(&mut value_lists);
            }
            data.map_values(&mut value_lists, &mut jump_tables, &mut exception_tables, |v| {
                *vmap.get(&v).unwrap_or(&v)
            });
            func.dfg.value_lists = value_lists;
            func.dfg.jump_tables = jump_tables;
            func.dfg.exception_tables = exception_tables;
        }
        let ctrl_typevar = func.dfg.ctrl_typevar(old_inst);
        let new_inst = func.dfg.make_inst(data);
        func.dfg.make_inst_results(new_inst, ctrl_typevar);
        let old_results = func.dfg.inst_results(old_inst).to_vec();
        let new_results = func.dfg.inst_results(new_inst).to_vec();
        for (o, n) in old_results.iter().zip(&new_results) {
            vmap.insert(*o, *n);
        }
        func.layout.append_inst(new_inst, dst);
    }
}

/// Rewrite the branch destination of a `jump`, keeping its arguments.
fn retarget(func: &mut ir::Function, inst: Inst, target: Block) {
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    if let ir::InstructionData::Jump { destination, .. } = &mut func.dfg.insts[inst] {
        let args: Vec<ir::BlockArg> = destination.args(&pool).collect();
        *destination = ir::BlockCall::new(target, args, &mut pool);
    }
    func.dfg.value_lists = pool;
}

/// A value expressed as `sum(coeffs[i] * header_param[i]) + c` over the
/// integers modulo `2^64`.
#[derive(Clone)]
struct Affine {
    coeffs: Vec<u64>,
    c: u64,
}

/// Compute the affine form of every instruction result in `plan.body`, starting
/// from the header parameters as the basis. Returns `None` if the body is not a
/// pure affine update of the loop state (it reads or writes memory, calls out, or
/// uses an operation we cannot model), which keeps the reassociation sound.
fn analyze_affine(func: &ir::Function, plan: &Plan) -> Option<Vec<Option<Affine>>> {
    let dfg = &func.dfg;
    let n = dfg.block_params(plan.header).len();

    // The closed form uses 64-bit arithmetic; a header carrying anything else
    // (a pointer, a float) is left alone.
    if !dfg.block_params(plan.header).iter().all(|&p| dfg.value_type(p) == ir::types::I64) {
        return None;
    }

    let zero = Affine { coeffs: vec![0; n], c: 0 };
    let basis = |i: usize| {
        let mut a = zero.clone();
        a.coeffs[i] = 1;
        a
    };
    // Only 64-bit induction/accumulator arithmetic is modelled, to keep the
    // wrapping semantics of the closed form exactly right.
    let is_i64 = |v: Value| dfg.value_type(dfg.resolve_aliases(v)) == ir::types::I64;

    let mut forms: Vec<Option<Affine>> = vec![None; dfg.num_values()];
    for (i, &p) in dfg.block_params(plan.header).iter().enumerate() {
        forms[p.as_u32() as usize] = Some(basis(i));
    }

    let add = |a: &Affine, b: &Affine| {
        let mut r = a.clone();
        for (x, y) in r.coeffs.iter_mut().zip(&b.coeffs) {
            *x = x.wrapping_add(*y);
        }
        r.c = r.c.wrapping_add(b.c);
        r
    };
    let scale = |a: &Affine, k: u64| {
        let mut r = a.clone();
        for x in r.coeffs.iter_mut() {
            *x = x.wrapping_mul(k);
        }
        r.c = r.c.wrapping_mul(k);
        r
    };
    let constant = |c: u64| Affine { coeffs: vec![0; n], c };
    let as_const = |a: &Option<Affine>| -> Option<u64> {
        let a = a.as_ref()?;
        if a.coeffs.iter().all(|&x| x == 0) {
            Some(a.c)
        } else {
            None
        }
    };

    for inst in func.layout.block_insts(plan.body) {
        let results = dfg.inst_results(inst).to_vec();
        let value = match dfg.insts[inst] {
            ir::InstructionData::UnaryImm { opcode: Opcode::Iconst, imm, .. }
                if is_i64(results[0]) =>
            {
                Some(constant(imm.bits() as u64))
            }
            ir::InstructionData::Binary { opcode: Opcode::Iadd, args, .. } if is_i64(args[0]) => {
                let (a, b) = (
                    forms[dfg.resolve_aliases(args[0]).as_u32() as usize].clone(),
                    forms[dfg.resolve_aliases(args[1]).as_u32() as usize].clone(),
                );
                match (a, b) {
                    (Some(a), Some(b)) => Some(add(&a, &b)),
                    _ => None,
                }
            }
            ir::InstructionData::Binary { opcode: Opcode::Imul, args, .. } if is_i64(args[0]) => {
                let (a, b) = (
                    forms[dfg.resolve_aliases(args[0]).as_u32() as usize].clone(),
                    forms[dfg.resolve_aliases(args[1]).as_u32() as usize].clone(),
                );
                match (as_const(&a), as_const(&b)) {
                    (Some(k), _) => b.map(|b| scale(&b, k)),
                    (_, Some(k)) => a.map(|a| scale(&a, k)),
                    _ => None,
                }
            }
            // Any memory access, call or unmodelled op aborts the rewrite.
            _ => None,
        };
        for r in results {
            forms[r.as_u32() as usize] = value.clone();
        }
    }
    Some(forms)
}

/// Compose the affine state update `factor` times and return the matrix mapping
/// the header parameters to their values after `factor` iterations.
fn power(func: &ir::Function, plan: &Plan, forms: &[Option<Affine>], factor: u64) -> Option<Vec<Affine>> {
    let dfg = &func.dfg;
    let n = dfg.block_params(plan.header).len();
    let back_args: Vec<Value> = match dfg.insts[plan.back_edge] {
        ir::InstructionData::Jump { destination, .. } => destination
            .args(&dfg.value_lists)
            .filter_map(|a| a.as_value())
            .collect(),
        _ => return None,
    };
    if back_args.len() != n {
        return None;
    }
    let step: Vec<Affine> = back_args
        .iter()
        .map(|&v| forms[dfg.resolve_aliases(v).as_u32() as usize].clone())
        .collect::<Option<Vec<_>>>()?;

    // Row `j` maps state -> state_j after one iteration. Iterate `factor` times.
    let mut rows = step.clone();
    for _ in 1..factor {
        let cur = rows.clone();
        rows = cur
            .iter()
            .map(|row| {
                let mut acc = Affine { coeffs: vec![0; n], c: row.c };
                for (i, &k) in row.coeffs.iter().enumerate() {
                    if k != 0 {
                        let mut scaled = step[i].clone();
                        for x in scaled.coeffs.iter_mut() {
                            *x = x.wrapping_mul(k);
                        }
                        scaled.c = scaled.c.wrapping_mul(k);
                        for (a, b) in acc.coeffs.iter_mut().zip(&scaled.coeffs) {
                            *a = a.wrapping_add(*b);
                        }
                        acc.c = acc.c.wrapping_add(scaled.c);
                    }
                }
                acc
            })
            .collect();
    }
    Some(rows)
}

fn apply(func: &mut ir::Function, plan: &Plan, factor: u64) {
    // The profitable transformation is affine reassociation: if the loop state
    // is a linear function of itself, its `factor`-fold iterate is also linear
    // and can be evaluated in a handful of multiplies, collapsing the carried
    // dependency chain. Duplicating the body cannot achieve that.
    if let Some(forms) = analyze_affine(func, plan) {
        if let Some(rows) = power(func, plan, &forms, factor) {
            apply_reassoc(func, plan, factor, &rows);
            return;
        }
    }
    if std::env::var_os("CG_CLIF_UNROLL_CLONE").is_some() {
        apply_clone(func, plan, factor);
    }
}

/// Rewrite the back edge so one header check may advance the loop state by
/// `factor` iterations at once, using the closed-form iterate `rows`, guarded so
/// a short tail falls back to the original single step.
fn apply_reassoc(func: &mut ir::Function, plan: &Plan, factor: u64, rows: &[Affine]) {
    let delta = plan.delta;
    let last = delta.wrapping_mul(factor - 1);
    let back_edge = plan.back_edge;

    // Arguments of the original back edge: already the single-step values.
    let single_args: Vec<ir::BlockArg> = match func.dfg.insts[back_edge] {
        ir::InstructionData::Jump { destination, .. } => {
            destination.args(&func.dfg.value_lists).collect()
        }
        _ => unreachable!(),
    };

    let header_params: Vec<Value> = func.dfg.block_params(plan.header).to_vec();
    let header = plan.header;

    // Build the guard and the composed state in front of the back edge.
    let (guard, wide_args) = {
        let mut pos = FuncCursor::new(func);
        pos.goto_inst(back_edge);

        // Wide path only when all `factor` iterations satisfy the loop test:
        // `iv + (factor-1)*delta` must still pass, and must not have wrapped.
        let iv = plan.iv;
        let iv_last = pos.ins().iadd_imm_s(iv, Imm64::new(last as i64));
        let no_wrap = pos.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, iv_last, iv);
        let last_ok = if plan.iv_left {
            pos.ins().icmp(plan.cc, iv_last, plan.bound)
        } else {
            pos.ins().icmp(plan.cc, plan.bound, iv_last)
        };
        let guard = pos.ins().band(no_wrap, last_ok);

        let mut wide_args = Vec::with_capacity(rows.len());
        for row in rows {
            let mut acc: Option<Value> = None;
            for (i, &k) in row.coeffs.iter().enumerate() {
                if k == 0 {
                    continue;
                }
                let term = if k == 1 {
                    header_params[i]
                } else {
                    let c = pos.ins().iconst(ir::types::I64, Imm64::new(k as i64));
                    pos.ins().imul(header_params[i], c)
                };
                acc = Some(match acc {
                    Some(a) => pos.ins().iadd(a, term),
                    None => term,
                });
            }
            let c = if row.c != 0 {
                Some(pos.ins().iconst(ir::types::I64, Imm64::new(row.c as i64)))
            } else {
                None
            };
            let value = match (acc, c) {
                (Some(a), Some(c)) => pos.ins().iadd(a, c),
                (Some(a), None) => a,
                (None, Some(c)) => c,
                (None, None) => pos.ins().iconst(ir::types::I64, Imm64::new(0)),
            };
            wide_args.push(ir::BlockArg::Value(value));
        }
        (guard, wide_args)
    };

    // Replace the back edge with a guard branch on the composed state.
    func.layout.remove_inst(back_edge);
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    let data = ir::InstructionData::Brif {
        opcode: Opcode::Brif,
        arg: guard,
        blocks: [
            ir::BlockCall::new(header, wide_args, &mut pool),
            ir::BlockCall::new(header, single_args, &mut pool),
        ],
    };
    let inst = func.dfg.make_inst(data);
    func.layout.append_inst(inst, plan.body);
    func.dfg.value_lists = pool;
}

/// Body-duplicating unroll: correct, but it does not shorten the carried
/// dependency chain, so it is only used as a fallback when explicitly enabled.
fn apply_clone(func: &mut ir::Function, plan: &Plan, factor: u64) {
    let delta = plan.delta;
    let last = delta.wrapping_mul(factor - 1);

    // Create `factor - 1` clone blocks with the header's parameter types.
    let header_tys: Vec<_> =
        func.dfg.block_params(plan.header).iter().map(|&p| func.dfg.value_type(p)).collect();
    let mut clones = Vec::new();
    for _ in 0..factor - 1 {
        let nb = func.dfg.make_block();
        func.layout.append_block(nb);
        for ty in &header_tys {
            func.dfg.append_block_param(nb, *ty);
        }
        clones.push(nb);
    }

    for &nb in &clones {
        clone_body(func, plan, nb);
    }

    // Guard at the top of the body: the wide path is taken only when all `factor`
    // iterations satisfy the loop test. `iv + last` must also not wrap, which
    // would otherwise make the test see a small value.
    let guard = {
        let iv = plan.iv;
        let bound = plan.bound;
        let mut pos = FuncCursor::new(func);
        pos.goto_first_insertion_point(plan.body);
        let iv_last = pos.ins().iadd_imm_s(iv, Imm64::new(last as i64));
        let no_wrap = pos.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, iv_last, iv);
        let last_ok = if plan.iv_left {
            pos.ins().icmp(plan.cc, iv_last, bound)
        } else {
            pos.ins().icmp(plan.cc, bound, iv_last)
        };
        pos.ins().band(no_wrap, last_ok)
    };

    // Chain the clones between the body and the header. Every terminator keeps
    // its arguments, which the body's own induction arithmetic has already
    // advanced, so the clones walk `iv`, `iv + delta`, ... automatically.
    for k in 0..clones.len() {
        let term = func.layout.last_inst(clones[k]).unwrap();
        if k + 1 < clones.len() {
            retarget(func, term, clones[k + 1]);
        }
    }
    let first = clones[0];
    let body_term = plan.back_edge;
    let body_args: Vec<ir::BlockArg> = match func.dfg.insts[body_term] {
        ir::InstructionData::Jump { destination, .. } => {
            destination.args(&func.dfg.value_lists).collect()
        }
        _ => unreachable!(),
    };

    // Replace the body's back edge with a guard branch: the wide path runs the
    // clones (covering `factor` iterations), the fallback takes a single step.
    func.layout.remove_inst(body_term);
    let mut pool = std::mem::take(&mut func.dfg.value_lists);
    let data = ir::InstructionData::Brif {
        opcode: Opcode::Brif,
        arg: guard,
        blocks: [
            ir::BlockCall::new(first, body_args.clone(), &mut pool),
            ir::BlockCall::new(plan.header, body_args, &mut pool),
        ],
    };
    let inst = func.dfg.make_inst(data);
    func.layout.append_inst(inst, plan.body);
    func.dfg.value_lists = pool;
}
