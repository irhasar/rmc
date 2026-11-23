//! Fuse `sinf(x)` + `cosf(x)` (or `sin`/`cos` for f64) with the same argument
//! into a single `sincosf`/`sincos` call, as LLVM does. The two libcalls share
//! the expensive argument reduction, so one `sincosf` is ~30% cheaper than
//! `sinf` + `cosf`. Only pairs where the sin call's block dominates the cos
//! call's block are fused, so the `sincosf` call and its two loads are
//! well-defined at every use of either result.
use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::{
    self, AbiParam, InstBuilder, MemFlagsData, StackSlotData, StackSlotKind, Type, Value,
};
use rakit_module::{Linkage, Module};

pub(crate) fn run(module: &mut dyn Module, func: &mut ir::Function) {
    if std::env::var_os("RAKIT_NO_SINCOS").is_some() {
        return;
    }
    let ptr_ty = module.isa().pointer_type();
    let call_conv = func.signature.call_conv;

    // Collect sin/cos call sites.
    struct Site {
        block: ir::Block,
        inst: ir::Inst,
        arg: Value,
        result: Value,
        ty: Type,
    }
    let mut sins: Vec<Site> = vec![];
    let mut coss: Vec<Site> = vec![];
    for block in func.layout.blocks().collect::<Vec<_>>() {
        for inst in func.layout.block_insts(block).collect::<Vec<_>>() {
            let ir::InstructionData::Call { func_ref, args, .. } = func.dfg.insts[inst] else {
                continue;
            };
            let Some(name) = call_name(module, func, func_ref) else { continue };
            let is_sin = name == "sinf" || name == "sin";
            let is_cos = name == "cosf" || name == "cos";
            if !is_sin && !is_cos {
                continue;
            }
            let Some(arg0) = args.get(0, &func.dfg.value_lists) else { continue };
            let Some(&result) = func.dfg.inst_results(inst).first() else { continue };
            let site = Site {
                block,
                inst,
                arg: func.dfg.resolve_aliases(arg0),
                result,
                ty: func.dfg.value_type(arg0),
            };
            if is_sin {
                sins.push(site)
            } else {
                coss.push(site)
            }
        }
    }
    if sins.is_empty() || coss.is_empty() {
        return;
    }

    let cfg = rakit_codegen::flowgraph::ControlFlowGraph::with_function(func);
    let mut dom = rakit_codegen::dominator_tree::DominatorTree::new();
    dom.compute(func, &cfg);

    // Pair each sin with a cos of the same argument whose block is dominated by
    // the sin's block. Collect pairs first so the DOM tree stays valid.
    let mut used = std::collections::HashSet::new();
    let mut pairs: Vec<(usize, usize)> = vec![];
    for (si, sin) in sins.iter().enumerate() {
        let Some((ci, _)) = coss.iter().enumerate().find(|(ci, cos)| {
            !used.contains(ci)
                && cos.arg == sin.arg
                && cos.ty == sin.ty
                && (cos.block == sin.block || dom.dominates(sin.block, cos.block, &func.layout))
        }) else {
            continue;
        };
        used.insert(ci);
        pairs.push((si, ci));
    }
    if pairs.is_empty() {
        return;
    }

    // Declare `sincosf` and/or `sincos` separately: a f32 pair and a f64 pair in
    // the same function need different callees.
    let mk_sig = |ty: Type| ir::Signature {
        params: vec![AbiParam::new(ty), AbiParam::new(ptr_ty), AbiParam::new(ptr_ty)],
        returns: vec![],
        call_conv,
    };
    let need_f32 = pairs.iter().any(|(si, _)| sins[*si].ty != ir::types::F64);
    let need_f64 = pairs.iter().any(|(si, _)| sins[*si].ty == ir::types::F64);
    let declare = |module: &mut dyn Module, func: &mut ir::Function, name: &str, ty: Type| {
        let sig = mk_sig(ty);
        module
            .declare_function(name, Linkage::Import, &sig)
            .ok()
            .map(|id| module.declare_func_in_func(id, func))
    };
    let f32_ref = if need_f32 { declare(module, func, "sincosf", ir::types::F32) } else { None };
    let f64_ref = if need_f64 { declare(module, func, "sincos", ir::types::F64) } else { None };
    let func_ref_of = |ty: Type| if ty == ir::types::F64 { f64_ref } else { f32_ref };

    for (si, ci) in pairs {
        let sin = &sins[si];
        let cos = &coss[ci];
        let Some(func_ref) = func_ref_of(sin.ty) else { continue };
        let bytes = sin.ty.bytes();
        let align_shift = bytes.trailing_zeros() as u8;
        let s_slot = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            bytes,
            align_shift,
        ));
        let c_slot = func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            bytes,
            align_shift,
        ));
        let mut pos = FuncCursor::new(func).at_inst(sin.inst);
        let s_addr = pos.ins().stack_addr(ptr_ty, s_slot, 0);
        let c_addr = pos.ins().stack_addr(ptr_ty, c_slot, 0);
        pos.ins().call(func_ref, &[sin.arg, s_addr, c_addr]);
        let s_val = pos.ins().load(sin.ty, MemFlagsData::trusted(), s_addr, 0);
        let c_val = pos.ins().load(sin.ty, MemFlagsData::trusted(), c_addr, 0);

        func.dfg.detach_inst_results(sin.inst);
        func.dfg.change_to_alias(sin.result, s_val);
        func.layout.remove_inst(sin.inst);

        func.dfg.detach_inst_results(cos.inst);
        func.dfg.change_to_alias(cos.result, c_val);
        func.layout.remove_inst(cos.inst);
    }
}

/// Recover the declared name of a direct call's callee.
fn call_name(module: &dyn Module, func: &ir::Function, func_ref: ir::FuncRef) -> Option<String> {
    let ir::ExternalName::User(reff) = func.dfg.ext_funcs[func_ref].name else {
        return None;
    };
    let index = func.params.user_named_funcs()[reff].index;
    let func_id = rakit_module::FuncId::from_u32(index);
    module.declarations().get_function_decl(func_id).name.clone()
}
