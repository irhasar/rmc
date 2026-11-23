//! codegen IR <-> RIR bridge for a conservative scalar subset.
//!
//! The bridge is intentionally partial: it accepts functions that use only
//! scalar constants, integer/float arithmetic, comparisons, `select`, block
//! parameters, `jump`/`brif` and `return`, and refuses everything else (calls,
//! memory, stack slots, globals, exception edges, multi-result instructions).
//! Refusing is always safe: the caller leaves the function untouched.

use std::collections::HashMap;

use rakit_codegen::ir::condcodes::{FloatCC as CgFloatCC, IntCC as CgIntCC};
use rakit_codegen::ir::immediates::{Ieee32, Ieee64, Imm64, Offset32};
use rakit_codegen::ir::{
    AbiParam, Block as CgBlock, BlockArg, BlockCall, DynamicStackSlot, DynamicStackSlotData, DynamicType,
    DynamicTypeData,
    ExceptionTableData, ExceptionTableItem, ExceptionTag, ExtFuncData, ExternalName,
    Function as CgFunction, GlobalValue, GlobalValueData, Inst as CgInst, InstructionData,
    KnownSymbol, LibCall, MemFlagsData, Opcode, Signature, StackSlot as CgStackSlot,
    StackSlotData as CgStackSlotData, StackSlotKind, TrapCode, Type as CgType, UserExternalName,
    Value as CgValue, ValueList, types,
};
use rakit_codegen::ir::{ArgumentExtension, ArgumentPurpose};
use rakit_codegen::isa::CallConv;
use rakit_ir::{
    BinOp, Block as RirBlock, Builder, DynamicSlotData, DynamicTypeData as RirDynamicTypeData,
    ExceptionItem as RirExceptionItem, External as RirExternal,
    ExternalTarget as RirExternalTarget, FloatCc as RirFloatCc, Function as RirFunction,
    GlobalData as RirGlobalData, Inst as RirInst, IntCc as RirIntCc,
    MemFlags as RirMemFlags, StackSlot as RirStackSlot, TryArg as RirTryArg,
    TryBlockCall as RirTryBlockCall,
    Type as RirType, UnOp as RirUnOp, Value as RirValue,
};

fn call_conv_to_rir(cc: CallConv) -> u8 {
    match cc {
        CallConv::Fast => 0,
        CallConv::Tail => 1,
        CallConv::SystemV => 2,
        CallConv::WindowsFastcall => 3,
        CallConv::AppleAarch64 => 4,
        CallConv::Probestack => 5,
        _ => 255,
    }
}

fn call_conv_from_rir(value: u8) -> Option<CallConv> {
    Some(match value {
        0 => CallConv::Fast,
        1 => CallConv::Tail,
        2 => CallConv::SystemV,
        3 => CallConv::WindowsFastcall,
        4 => CallConv::AppleAarch64,
        5 => CallConv::Probestack,
        _ => return None,
    })
}

fn memory_flags_supported(func: &CgFunction, flags: rakit_codegen::ir::MemFlags) -> bool {
    func.dfg.mem_flags[flags].explicit_endianness().is_none()
}

fn abi_params_supported(sig: &Signature) -> bool {
    sig.params.iter().chain(sig.returns.iter()).all(|p| {
        p.purpose == ArgumentPurpose::Normal && p.extension == ArgumentExtension::None
    })
}

fn signature_supported(sig: &Signature) -> bool {
    sig.call_conv == CallConv::SystemV && abi_params_supported(sig)
}

/// Indirect calls carry their calling convention through RIR, so any convention
/// the bridge can encode is safe as long as the ABI parameters are ordinary.
fn indirect_signature_supported(sig: &Signature) -> bool {
    call_conv_to_rir(sig.call_conv) != 255 && abi_params_supported(sig)
}

/// Import a codegen IR function into RIR, or `None` if it uses anything outside the
/// supported scalar subset.
pub fn import(func: &CgFunction) -> Option<RirFunction> {
    let entry = func.layout.entry_block()?;
    let ret: Vec<RirType> = func
        .signature
        .returns
        .iter()
        .map(|p| map_type_to_rir(p.value_type))
        .collect::<Option<_>>()?;
    let entry_params: Vec<RirType> = func
        .dfg
        .block_params(entry)
        .iter()
        .map(|v| map_type_to_rir(func.dfg.value_type(*v)))
        .collect::<Option<_>>()?;

    if func.signature.params.len() != entry_params.len() {
        return None;
    }
    for (p, t) in func.signature.params.iter().zip(&entry_params) {
        if map_type_to_rir(p.value_type)? != *t {
            return None;
        }
    }

    let mut builder = Builder::new(ret.clone(), entry_params);
    let mut externals: Vec<RirExternal> = Vec::new();
    let mut slot_map: HashMap<CgStackSlot, RirStackSlot> = HashMap::new();
    for (cs, data) in func.sized_stack_slots.iter() {
        if !matches!(data.kind, StackSlotKind::ExplicitSlot) {
            return None;
        }
        let align = 1u32.checked_shl(data.align_shift as u32).unwrap_or(1);
        let rs = builder.add_stack_slot(data.size, align);
        slot_map.insert(cs, rs);
    }
    let mut globals = Vec::with_capacity(func.global_values.len());
    for (_, data) in func.global_values.iter() {
        globals.push(map_global_data(func, data)?);
    }
    let mut dynamic_types = Vec::with_capacity(func.dfg.dynamic_types.len());
    for (_, data) in func.dfg.dynamic_types.iter() {
        dynamic_types.push(RirDynamicTypeData {
            base_vector_ty: data.base_vector_ty.as_u32(),
            dynamic_scale: data.dynamic_scale.as_u32(),
        });
    }
    let mut dynamic_stack_slots = Vec::with_capacity(func.dynamic_stack_slots.len());
    for (_, data) in func.dynamic_stack_slots.iter() {
        if !matches!(data.kind, StackSlotKind::ExplicitDynamicSlot) {
            return None;
        }
        dynamic_stack_slots.push(DynamicSlotData {
            kind: 0,
            dyn_ty: data.dyn_ty.as_u32(),
        });
    }
    let stack_limit = func.stack_limit.map(|g| g.as_u32());
    let mut block_map: HashMap<CgBlock, RirBlock> = HashMap::new();
    block_map.insert(entry, RirBlock(0));

    for b in func.layout.blocks() {
        if b == entry {
            continue;
        }
        let params: Vec<RirType> = func
            .dfg
            .block_params(b)
            .iter()
            .map(|v| map_type_to_rir(func.dfg.value_type(*v)))
            .collect::<Option<_>>()?;
        let rb = builder.add_block(params);
        block_map.insert(b, rb);
    }

    let mut vmap: HashMap<CgValue, RirValue> = HashMap::new();
    for (cb, rb) in &block_map {
        let cparams = func.dfg.block_params(*cb);
        let rparams = builder.func().block(*rb).params.clone();
        if cparams.len() != rparams.len() {
            return None;
        }
        for (c, r) in cparams.iter().zip(rparams) {
            map_value(func, &mut vmap, *c, r);
        }
    }

    for b in func.layout.blocks() {
        let mut rb = *block_map.get(&b)?;
        for inst in func.layout.block_insts(b) {
            let data = func.dfg.insts[inst];
            match data {
                InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    if ty == RirType::I128 {
                        return None;
                    }
                    let v = builder.iconst(rb, ty, imm.bits());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::UnaryIeee32 { opcode: Opcode::F32const, imm } => {
                    let r = single_result(func, inst)?;
                    let v = builder.f32const(rb, imm.as_f32());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::UnaryIeee64 { opcode: Opcode::F64const, imm } => {
                    let r = single_result(func, inst)?;
                    let v = builder.f64const(rb, imm.as_f64());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Unary { opcode, arg } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let x = if opcode == Opcode::Sextend {
                        as_width(&mut builder, rb, lookup(func, &vmap, arg)?, ty, true)
                    } else {
                        as_int(&mut builder, rb, lookup(func, &vmap, arg)?, ty)
                    };
                    let v = if opcode == Opcode::Ineg {
                        let zero = builder.iconst(rb, ty, 0);
                        builder.bin(rb, BinOp::Sub, ty, zero, x)
                    } else {
                        let op = map_unop(opcode)?;
                        builder.unary(rb, op, ty, x)
                    };
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Binary { opcode, args } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let signed = matches!(opcode, Opcode::Sdiv | Opcode::Srem | Opcode::Sshr);
                    let x = as_width(&mut builder, rb, lookup(func, &vmap, args[0])?, ty, signed);
                    let y = if matches!(opcode, Opcode::Umin | Opcode::Umax) {
                        as_int(&mut builder, rb, lookup(func, &vmap, args[1])?, ty)
                    } else {
                        let op = map_binop(opcode)?;
                        if matches!(op, BinOp::Shl | BinOp::Shr | BinOp::Ushr) {
                            as_width(&mut builder, rb, lookup(func, &vmap, args[1])?, ty, false)
                        } else {
                            as_width(&mut builder, rb, lookup(func, &vmap, args[1])?, ty, signed)
                        }
                    };
                    let v = match opcode {
                        Opcode::Umin => {
                            let c = builder.icmp(rb, RirIntCc::Ult, ty, x, y);
                            builder.select(rb, ty, c, x, y)
                        }
                        Opcode::Umax => {
                            let c = builder.icmp(rb, RirIntCc::Ugt, ty, x, y);
                            builder.select(rb, ty, c, x, y)
                        }
                        _ => {
                            let op = map_binop(opcode)?;
                            builder.bin(rb, op, ty, x, y)
                        }
                    };
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::IntCompare { opcode: Opcode::Icmp, args, cond } => {
                    let cc = map_intcc(cond)?;
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    let signed = matches!(cc, RirIntCc::Slt | RirIntCc::Sle | RirIntCc::Sgt | RirIntCc::Sge);
                    let x = as_width(&mut builder, rb, lookup(func, &vmap, args[0])?, ty, signed);
                    let y = as_width(&mut builder, rb, lookup(func, &vmap, args[1])?, ty, signed);
                    let v = builder.icmp(rb, cc, ty, x, y);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::FloatCompare { opcode: Opcode::Fcmp, args, cond } => {
                    let cc = map_floatcc(cond)?;
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    let x = lookup(func, &vmap, args[0])?;
                    let y = lookup(func, &vmap, args[1])?;
                    let v = builder.fcmp(rb, cc, ty, x, y);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Ternary { opcode: Opcode::Select, args } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let c = as_bool(&mut builder, rb, lookup(func, &vmap, args[0])?);
                    let x = as_int(&mut builder, rb, lookup(func, &vmap, args[1])?, ty);
                    let y = as_int(&mut builder, rb, lookup(func, &vmap, args[2])?, ty);
                    let v = builder.select(rb, ty, c, x, y);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::StackAddr { opcode: Opcode::StackAddr, stack_slot, offset } => {
                    let r = single_result(func, inst)?;
                    let slot = *slot_map.get(&stack_slot)?;
                    let v = builder.stack_addr(rb, slot, offset.into());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Load { opcode: Opcode::Load, arg, offset, flags } => {
                    if !memory_flags_supported(func, flags) {
                        return None;
                    }
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let addr = lookup(func, &vmap, arg)?;
                    let flags = RirMemFlags(func.dfg.mem_flags[flags].bits());
                    let v = builder.load_with_flags(rb, ty, addr, offset.into(), flags);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::LoadNoOffset { opcode: Opcode::Load, arg, flags } => {
                    if !memory_flags_supported(func, flags) {
                        return None;
                    }
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let addr = lookup(func, &vmap, arg)?;
                    let flags = RirMemFlags(func.dfg.mem_flags[flags].bits());
                    let v = builder.load_with_flags(rb, ty, addr, 0, flags);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Store { opcode: Opcode::Store, args, offset, flags } => {
                    if !memory_flags_supported(func, flags) {
                        return None;
                    }
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    let value = as_int(&mut builder, rb, lookup(func, &vmap, args[0])?, ty);
                    let addr = lookup(func, &vmap, args[1])?;
                    let flags = RirMemFlags(func.dfg.mem_flags[flags].bits());
                    builder.store_with_flags(rb, ty, addr, offset.into(), value, flags);
                }
                InstructionData::StoreNoOffset { opcode: Opcode::Store, args, flags } => {
                    if !memory_flags_supported(func, flags) {
                        return None;
                    }
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    let value = as_int(&mut builder, rb, lookup(func, &vmap, args[0])?, ty);
                    let addr = lookup(func, &vmap, args[1])?;
                    let flags = RirMemFlags(func.dfg.mem_flags[flags].bits());
                    builder.store_with_flags(rb, ty, addr, 0, value, flags);
                }
                InstructionData::Call { func_ref, args, .. } => {
                    let ext_data = &func.dfg.ext_funcs[func_ref];
                    let callee = &func.dfg.signatures[ext_data.signature];
                    if !signature_supported(callee) {
                        return None;
                    }
                    let params = callee
                        .params
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let rets = callee
                        .returns
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let target = match ext_data.name {
                        ExternalName::User(name_ref) => {
                            let name = &func.params.user_named_funcs()[name_ref];
                            RirExternalTarget::User { namespace: name.namespace, index: name.index }
                        }
                        ExternalName::LibCall(call) => RirExternalTarget::LibCall(call.to_string()),
                        ExternalName::KnownSymbol(symbol) => {
                            RirExternalTarget::KnownSymbol(symbol.to_string())
                        }
                        _ => return None,
                    };
                    let external = RirExternal { target, params: params.clone(), rets: rets.clone() };
                    let target = match externals.iter().position(|e| *e == external) {
                        Some(i) => i as u32,
                        None => {
                            externals.push(external);
                            (externals.len() - 1) as u32
                        }
                    };
                    let args = args
                        .as_slice(&func.dfg.value_lists)
                        .iter()
                        .zip(&params)
                        .map(|(v, ty)| {
                            lookup(func, &vmap, *v).map(|v| as_int(&mut builder, rb, v, *ty))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let results: Vec<_> = func.dfg.inst_results(inst).to_vec();
                    let rets = results
                        .iter()
                        .map(|r| map_type_to_rir(func.dfg.value_type(*r)))
                        .collect::<Option<Vec<_>>>()?;
                    let rir_results = builder.call_multi(rb, target, args, rets);
                    for (cg, rir) in results.iter().zip(rir_results) {
                        map_value(func, &mut vmap, *cg, rir);
                    }
                }
                InstructionData::CallIndirect { sig_ref, args, .. } => {
                    let callee = args.as_slice(&func.dfg.value_lists)[0];
                    let callee = lookup(func, &vmap, callee)?;
                    let callee_sig = &func.dfg.signatures[sig_ref];
                    if !indirect_signature_supported(callee_sig) {
                        return None;
                    }
                    let params = callee_sig
                        .params
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let call_args = args.as_slice(&func.dfg.value_lists)[1..]
                        .iter()
                        .zip(&params)
                        .map(|(v, ty)| {
                            lookup(func, &vmap, *v).map(|v| as_int(&mut builder, rb, v, *ty))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    let rets = callee_sig
                        .returns
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let results: Vec<_> = func.dfg.inst_results(inst).to_vec();
                    let result_types = results
                        .iter()
                        .map(|r| map_type_to_rir(func.dfg.value_type(*r)))
                        .collect::<Option<Vec<_>>>()?;
                    if result_types != rets {
                        return None;
                    }
                    let rir_results = builder.call_indirect(
                        rb,
                        params,
                        rets,
                        call_conv_to_rir(callee_sig.call_conv),
                        callee,
                        call_args,
                    );
                    for (cg, rir) in results.iter().zip(rir_results) {
                        map_value(func, &mut vmap, *cg, rir);
                    }
                }
                InstructionData::UnaryGlobalValue { global_value, .. } => {
                    let r = single_result(func, inst)?;
                    let v = builder.global_value(rb, global_value.as_u32());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::FuncAddr { opcode: Opcode::FuncAddr, func_ref } => {
                    let r = single_result(func, inst)?;
                    let target = external_index(func, &mut externals, &func.dfg.ext_funcs[func_ref])?;
                    let v = builder.func_addr(rb, target);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::DynamicStackAddr {
                    opcode: Opcode::DynamicStackAddr,
                    dynamic_stack_slot,
                } => {
                    let r = single_result(func, inst)?;
                    let v = builder.dynamic_stack_addr(rb, dynamic_stack_slot.as_u32());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::TryCall { func_ref, args, exception, .. } => {
                    let ext_data = &func.dfg.ext_funcs[func_ref];
                    if !signature_supported(&func.dfg.signatures[ext_data.signature]) {
                        return None;
                    }
                    let target = external_index(func, &mut externals, ext_data)?;
                    let args = args
                        .as_slice(&func.dfg.value_lists)
                        .iter()
                        .map(|v| lookup(func, &vmap, *v))
                        .collect::<Option<Vec<_>>>()?;
                    let results: Vec<_> = func.dfg.inst_results(inst).to_vec();
                    let table = &func.dfg.exception_tables[exception];
                    if !signature_supported(&func.dfg.signatures[table.signature()]) {
                        return None;
                    }
                    let rets = func.dfg.signatures[table.signature()]
                        .returns
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let normal = map_try_block(func, table.normal_return(), &vmap, &block_map)?;
                    let exception = map_exception(func, table, &vmap, &block_map)?;
                    let rir_results = builder.try_call(rb, target, args, rets, normal, exception);
                    for (cg, rir) in results.iter().zip(rir_results) {
                        map_value(func, &mut vmap, *cg, rir);
                    }
                }
                InstructionData::TryCallIndirect { args, exception, .. } => {
                    let all = args.as_slice(&func.dfg.value_lists);
                    let callee = lookup(func, &vmap, all[0])?;
                    let call_args = all[1..]
                        .iter()
                        .map(|v| lookup(func, &vmap, *v))
                        .collect::<Option<Vec<_>>>()?;
                    let results: Vec<_> = func.dfg.inst_results(inst).to_vec();
                    let table = &func.dfg.exception_tables[exception];
                    if !indirect_signature_supported(&func.dfg.signatures[table.signature()]) {
                        return None;
                    }
                    let rets = func.dfg.signatures[table.signature()]
                        .returns
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let params = func.dfg.signatures[table.signature()]
                        .params
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
                        .collect::<Option<Vec<_>>>()?;
                    let normal = map_try_block(func, table.normal_return(), &vmap, &block_map)?;
                    let exception = map_exception(func, table, &vmap, &block_map)?;
                    let rir_results = builder.try_call_indirect(
                        rb,
                        params,
                        rets,
                        call_conv_to_rir(func.dfg.signatures[table.signature()].call_conv),
                        callee,
                        call_args,
                        normal,
                        exception,
                    );
                    for (cg, rir) in results.iter().zip(rir_results) {
                        map_value(func, &mut vmap, *cg, rir);
                    }
                }
                InstructionData::CondTrap { opcode, arg, code } => {
                    let ty = map_type_to_rir(func.dfg.value_type(arg))?;
                    let x = as_int(&mut builder, rb, lookup(func, &vmap, arg)?, ty);
                    let zero = builder.iconst(rb, ty, 0);
                    let cc = match opcode {
                        Opcode::Trapz => RirIntCc::Eq,
                        Opcode::Trapnz => RirIntCc::Ne,
                        _ => return None,
                    };
                    let cond = builder.icmp(rb, cc, ty, x, zero);
                    let trap_block = builder.add_block(Vec::new());
                    let cont_block = builder.add_block(Vec::new());
                    builder.brif(rb, cond, trap_block, Vec::new(), cont_block, Vec::new());
                    builder.trap(trap_block, code.as_raw().get());
                    rb = cont_block;
                }
                InstructionData::BranchTable { arg, table, .. } => {
                    let ty = map_type_to_rir(func.dfg.value_type(arg))?;
                    let discr = lookup(func, &vmap, arg)?;
                    let table = &func.dfg.jump_tables[table];
                    let default = table.default_block();
                    let default_block = *block_map.get(&default.block(&func.dfg.value_lists))?;
                    let default_args =
                        typed_block_args(func, &default, &vmap, &mut builder, rb, default_block)?;
                    let entries = table.as_slice().to_vec();
                    let mut current = rb;
                    for (index, entry) in entries.iter().enumerate() {
                        let target = *block_map.get(&entry.block(&func.dfg.value_lists))?;
                        let args = typed_block_args(func, entry, &vmap, &mut builder, current, target)?;
                        let value = builder.iconst(current, ty, index as i64);
                        let cond = builder.icmp(current, RirIntCc::Eq, ty, discr, value);
                        let next = builder.add_block(Vec::new());
                        builder.brif(current, cond, target, args, next, Vec::new());
                        current = next;
                    }
                    builder.jump(current, default_block, default_args);
                }
                InstructionData::Jump { destination, .. } => {
                    let target = *block_map.get(&destination.block(&func.dfg.value_lists))?;
                    let args = typed_block_args(func, &destination, &vmap, &mut builder, rb, target)?;
                    builder.jump(rb, target, args);
                }
                InstructionData::Brif { arg, blocks, .. } => {
                    let c = as_bool(&mut builder, rb, lookup(func, &vmap, arg)?);
                    let then_block = *block_map.get(&blocks[0].block(&func.dfg.value_lists))?;
                    let then_args =
                        typed_block_args(func, &blocks[0], &vmap, &mut builder, rb, then_block)?;
                    let else_block = *block_map.get(&blocks[1].block(&func.dfg.value_lists))?;
                    let else_args =
                        typed_block_args(func, &blocks[1], &vmap, &mut builder, rb, else_block)?;
                    builder.brif(rb, c, then_block, then_args, else_block, else_args);
                }
                InstructionData::MultiAry { opcode: Opcode::Return, args } => {
                    let vals = args
                        .as_slice(&func.dfg.value_lists)
                        .iter()
                        .zip(&ret)
                        .map(|(v, ty)| {
                            lookup(func, &vmap, *v).map(|v| as_int(&mut builder, rb, v, *ty))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    builder.ret(rb, vals);
                }
                InstructionData::NullAry { opcode: Opcode::Nop } => {}
                InstructionData::Trap { code, .. } => {
                    if std::env::var_os("RAKIT_NO_RIR_TRAP").is_some() {
                        return None;
                    }
                    builder.trap(rb, code.as_raw().get());
                }
                _ => {
                    if std::env::var_os("RAKIT_RIR_DEBUG").is_some() {
                        eprintln!(
                            "[rir] unsupported inst {} in {:?}",
                            data.opcode(),
                            func.name
                        );
                    }
                    return None;
                }
            }
        }
    }

    let mut out = builder.into_func();
    out.self_target = match &func.name {
        rakit_codegen::ir::UserFuncName::User(name) => Some(RirExternalTarget::User {
            namespace: name.namespace,
            index: name.index,
        }),
        _ => None,
    };
    out.externals = externals;
    out.globals = globals;
    out.dynamic_types = dynamic_types;
    out.dynamic_stack_slots = dynamic_stack_slots;
    out.stack_limit = stack_limit;
    if std::env::var_os("RAKIT_NO_RIR_IMPORT_VERIFY").is_none() {
        if let Err(errors) = rakit_ir::verify(&out) {
            if std::env::var_os("RAKIT_RIR_IMPORT_DEBUG").is_some() {
                eprintln!("[rir] import verify failed in {:?}: {errors:?}", func.name);
            }
            if std::env::var_os("RAKIT_RIR_IMPORT_DUMP").is_some() {
                eprintln!("[rir] backend IR\n{}", func.display());
                eprintln!("[rir] RIR\n{out:?}");
            }
            return None;
        }
    }
    Some(out)
}

/// Lower an RIR function back to codegen IR, reusing `sig` for the calling convention
/// and ABI parameter details. Returns `None` on any inconsistency.
fn emit_i128_const(func: &mut CgFunction, cb: CgBlock, value: i64, signed: bool) -> Option<CgValue> {
    let lo = func.dfg.make_inst(InstructionData::UnaryImm {
        opcode: Opcode::Iconst,
        imm: Imm64::new(value),
    });
    if func.dfg.make_inst_results(lo, types::I64) != 1 {
        return None;
    }
    func.layout.append_inst(lo, cb);
    let lo = func.dfg.first_result(lo);
    let ext = func.dfg.make_inst(InstructionData::Unary {
        opcode: if signed { Opcode::Sextend } else { Opcode::Uextend },
        arg: lo,
    });
    if func.dfg.make_inst_results(ext, types::I128) != 1 {
        return None;
    }
    func.layout.append_inst(ext, cb);
    Some(func.dfg.first_result(ext))
}

pub fn export(rir: &RirFunction, sig: &Signature) -> Option<CgFunction> {
    let entry_params = &rir.block(rir.entry).params;
    if sig.params.len() != entry_params.len() || sig.returns.len() != rir.ret.len() {
        return None;
    }
    for (p, rv) in sig.params.iter().zip(entry_params) {
        if map_type_to_cg(rir.ty(*rv))? != p.value_type {
            return None;
        }
    }
    for (p, t) in sig.returns.iter().zip(&rir.ret) {
        if map_type_to_cg(*t)? != p.value_type {
            return None;
        }
    }

    let mut func = CgFunction::new();
    func.signature = sig.clone();

    let mut call_refs = Vec::with_capacity(rir.externals.len());
    for ext in &rir.externals {
        let mut callee = Signature::new(CallConv::SystemV);
        for ty in &ext.params {
            callee.params.push(AbiParam::new(map_type_to_cg(*ty)?));
        }
        for ty in &ext.rets {
            callee.returns.push(AbiParam::new(map_type_to_cg(*ty)?));
        }
        let sig_ref = func.import_signature(callee);
        let name = match &ext.target {
            RirExternalTarget::User { namespace, index } => {
                let name_ref =
                    func.params.ensure_user_func_name(UserExternalName::new(*namespace, *index));
                ExternalName::User(name_ref)
            }
            RirExternalTarget::LibCall(name) => {
                ExternalName::LibCall(name.parse::<LibCall>().ok()?)
            }
            RirExternalTarget::KnownSymbol(name) => {
                ExternalName::KnownSymbol(name.parse::<KnownSymbol>().ok()?)
            }
        };
        let data = ExtFuncData { name, signature: sig_ref, colocated: false, patchable: false };
        call_refs.push(func.import_function(data));
    }

    for global in &rir.globals {
        let data = match global {
            RirGlobalData::VMContext => GlobalValueData::VMContext,
            RirGlobalData::Load { base, offset, ty, .. } => GlobalValueData::Load {
                base: GlobalValue::from_u32(*base),
                offset: Offset32::try_from_i64(*offset)?,
                global_type: map_type_to_cg(*ty)?,
                flags: func.dfg.mem_flags.insert_unchecked(MemFlagsData::trusted()),
            },
            RirGlobalData::IAddImm { base, offset, ty } => GlobalValueData::IAddImm {
                base: GlobalValue::from_u32(*base),
                offset: Imm64::new(*offset),
                global_type: map_type_to_cg(*ty)?,
            },
            RirGlobalData::Symbol { target, offset, colocated, tls } => GlobalValueData::Symbol {
                name: external_name(&mut func, target)?,
                offset: Imm64::new(*offset),
                colocated: *colocated,
                tls: *tls,
            },
            RirGlobalData::DynScaleTargetConst { vector_type } => {
                GlobalValueData::DynScaleTargetConst {
                    vector_type: CgType::from_bits(*vector_type),
                }
            }
        };
        func.create_global_value(data);
    }
    func.stack_limit = rir.stack_limit.map(GlobalValue::from_u32);
    for ty in &rir.dynamic_types {
        let base = CgType::from_bits(ty.base_vector_ty);
        let scale = GlobalValue::from_u32(ty.dynamic_scale);
        func.dfg.dynamic_types.push(DynamicTypeData::new(base, scale));
    }
    for slot in &rir.dynamic_stack_slots {
        let dyn_ty = DynamicType::from_u32(slot.dyn_ty);
        let data = DynamicStackSlotData::new(StackSlotKind::ExplicitDynamicSlot, dyn_ty);
        func.create_dynamic_stack_slot(data);
    }

    let mut slot_map: Vec<CgStackSlot> = Vec::with_capacity(rir.stack_slots.len());
    for slot in &rir.stack_slots {
        let align_shift = slot.align.max(1).trailing_zeros() as u8;
        let data = CgStackSlotData::new(StackSlotKind::ExplicitSlot, slot.size, align_shift);
        slot_map.push(func.create_sized_stack_slot(data));
    }

    let entry_idx = rir.entry.0 as usize;
    if entry_idx >= rir.blocks.len() {
        return None;
    }
    let mut order = Vec::with_capacity(rir.blocks.len());
    order.push(entry_idx);
    order.extend((0..rir.blocks.len()).filter(|&i| i != entry_idx));
    let mut block_map: Vec<CgBlock> = Vec::with_capacity(rir.blocks.len());
    for _ in 0..rir.blocks.len() {
        block_map.push(func.dfg.make_block());
    }
    let mut vmap: HashMap<RirValue, CgValue> = HashMap::new();
    for &bi in &order {
        let rb = &rir.blocks[bi];
        let cb = block_map[bi];
        func.layout.append_block(cb);
        for p in &rb.params {
            let ty = map_type_to_cg(rir.ty(*p))?;
            let cv = func.dfg.append_block_param(cb, ty);
            vmap.insert(*p, cv);
        }
    }

    for &bi in &order {
        let rb = &rir.blocks[bi];
        let cb = block_map[bi];
        for inst in &rb.insts {
            match inst {
                RirInst::Nop(r, ty) => {
                    if *ty == RirType::I128 {
                        let v = emit_i128_const(&mut func, cb, 0, false)?;
                        vmap.insert(*r, v);
                        continue;
                    }
                    let ct = map_type_to_cg(*ty)?;
                    let data = match ty {
                        RirType::F32 => InstructionData::UnaryIeee32 {
                            opcode: Opcode::F32const,
                            imm: Ieee32::with_float(0.0),
                        },
                        RirType::F64 => InstructionData::UnaryIeee64 {
                            opcode: Opcode::F64const,
                            imm: Ieee64::with_float(0.0),
                        },
                        _ => {
                            InstructionData::UnaryImm { opcode: Opcode::Iconst, imm: Imm64::new(0) }
                        }
                    };
                    let ci = func.dfg.make_inst(data);
                    if func.dfg.make_inst_results(ci, ct) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Iconst(r, ty, v) => {
                    if *ty == RirType::I128 {
                        let v = emit_i128_const(&mut func, cb, *v, true)?;
                        vmap.insert(*r, v);
                        continue;
                    }
                    if !matches!(
                        ty,
                        RirType::I8 | RirType::I16 | RirType::I32 | RirType::I64
                            | RirType::Bool | RirType::Ptr
                    ) {
                        return None;
                    }
                    let ct = map_type_to_cg(*ty)?;
                    let ci = func.dfg.make_inst(InstructionData::UnaryImm {
                        opcode: Opcode::Iconst,
                        imm: Imm64::new(*v),
                    });
                    if func.dfg.make_inst_results(ci, ct) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::F32const(r, v) => {
                    let ci = func.dfg.make_inst(InstructionData::UnaryIeee32 {
                        opcode: Opcode::F32const,
                        imm: Ieee32::with_float(*v),
                    });
                    if func.dfg.make_inst_results(ci, types::F32) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::F64const(r, v) => {
                    let ci = func.dfg.make_inst(InstructionData::UnaryIeee64 {
                        opcode: Opcode::F64const,
                        imm: Ieee64::with_float(*v),
                    });
                    if func.dfg.make_inst_results(ci, types::F64) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Copy(r, _, x) => {
                    let cv = *vmap.get(x)?;
                    vmap.insert(*r, cv);
                }
                RirInst::Unary(r, op, ty, x) => {
                    let ct = map_type_to_cg(*ty)?;
                    let arg = *vmap.get(x)?;
                    let at = func.dfg.value_type(arg);
                    let opcode = match op {
                        RirUnOp::Uextend | RirUnOp::Sextend | RirUnOp::Ireduce => {
                            if at == ct {
                                vmap.insert(*r, arg);
                                continue;
                            }
                            let widening = at.bits() < ct.bits();
                            match op {
                                RirUnOp::Uextend if widening => Opcode::Uextend,
                                RirUnOp::Sextend if widening => Opcode::Sextend,
                                RirUnOp::Ireduce if !widening => Opcode::Ireduce,
                                _ => return None,
                            }
                        }
                        _ => map_unop_to_cg(*op)?,
                    };
                    let ci = func.dfg.make_inst(InstructionData::Unary {
                        opcode,
                        arg,
                    });
                    if func.dfg.make_inst_results(ci, ct) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Bin(r, op, ty, x, y) => {
                    let ct = map_type_to_cg(*ty)?;
                    let ci = func.dfg.make_inst(InstructionData::Binary {
                        opcode: map_binop_to_cg(*op)?,
                        args: [*vmap.get(x)?, *vmap.get(y)?],
                    });
                    if func.dfg.make_inst_results(ci, ct) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Icmp(r, cc, _, x, y) => {
                    let ci = func.dfg.make_inst(InstructionData::IntCompare {
                        opcode: Opcode::Icmp,
                        args: [*vmap.get(x)?, *vmap.get(y)?],
                        cond: map_intcc_to_cg(*cc)?,
                    });
                    if func.dfg.make_inst_results(ci, types::I8) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Fcmp(r, cc, _, x, y) => {
                    let ci = func.dfg.make_inst(InstructionData::FloatCompare {
                        opcode: Opcode::Fcmp,
                        args: [*vmap.get(x)?, *vmap.get(y)?],
                        cond: map_floatcc_to_cg(*cc)?,
                    });
                    if func.dfg.make_inst_results(ci, types::I8) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Select(r, ty, c, x, y) => {
                    let ct = map_type_to_cg(*ty)?;
                    let ci = func.dfg.make_inst(InstructionData::Ternary {
                        opcode: Opcode::Select,
                        args: [*vmap.get(c)?, *vmap.get(x)?, *vmap.get(y)?],
                    });
                    if func.dfg.make_inst_results(ci, ct) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::StackAddr(r, slot, offset) => {
                    let cs = *slot_map.get(slot.0 as usize)?;
                    let ci = func.dfg.make_inst(InstructionData::StackAddr {
                        opcode: Opcode::StackAddr,
                        stack_slot: cs,
                        offset: (*offset).into(),
                    });
                    if func.dfg.make_inst_results(ci, types::I64) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Load(r, ty, addr, offset, flags) => {
                    let ct = map_type_to_cg(*ty)?;
                    let flags = func
                        .dfg
                        .mem_flags
                        .insert(MemFlagsData::from_bits(flags.0))
                        .ok()?;
                    let ci = func.dfg.make_inst(InstructionData::Load {
                        opcode: Opcode::Load,
                        flags,
                        arg: *vmap.get(addr)?,
                        offset: (*offset).into(),
                    });
                    if func.dfg.make_inst_results(ci, ct) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Store(ty, addr, offset, value, flags) => {
                    let _ = map_type_to_cg(*ty)?;
                    let flags = func
                        .dfg
                        .mem_flags
                        .insert(MemFlagsData::from_bits(flags.0))
                        .ok()?;
                    let ci = func.dfg.make_inst(InstructionData::Store {
                        opcode: Opcode::Store,
                        flags,
                        args: [*vmap.get(value)?, *vmap.get(addr)?],
                        offset: (*offset).into(),
                    });
                    func.layout.append_inst(ci, cb);
                }
                RirInst::Call(results, target, args) => {
                    let func_ref = *call_refs.get(*target as usize)?;
                    let mut list = ValueList::default();
                    for v in args {
                        list.push(*vmap.get(v)?, &mut func.dfg.value_lists);
                    }
                    let ci = func.dfg.make_inst(InstructionData::Call {
                        opcode: Opcode::Call,
                        func_ref,
                        args: list,
                    });
                    let ext = &rir.externals[*target as usize];
                    let ctrl = match ext.rets.first() {
                        Some(ty) => map_type_to_cg(*ty)?,
                        None => types::I8,
                    };
                    let count = func.dfg.make_inst_results(ci, ctrl);
                    if count != ext.rets.len() || count != results.len() {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    let cg_results: Vec<_> = func.dfg.inst_results(ci).to_vec();
                    for (rir, cg) in results.iter().zip(cg_results) {
                        vmap.insert(*rir, cg);
                    }
                }
                RirInst::CallIndirect { results, params, rets, call_conv, callee, args } => {
                    let mut callee_sig = Signature::new(call_conv_from_rir(*call_conv)?);
                    for ty in params {
                        callee_sig.params.push(AbiParam::new(map_type_to_cg(*ty)?));
                    }
                    for ty in rets {
                        callee_sig.returns.push(AbiParam::new(map_type_to_cg(*ty)?));
                    }
                    let sig_ref = func.import_signature(callee_sig);
                    let mut list = ValueList::default();
                    list.push(*vmap.get(callee)?, &mut func.dfg.value_lists);
                    for arg in args {
                        list.push(*vmap.get(arg)?, &mut func.dfg.value_lists);
                    }
                    let ci = func.dfg.make_inst(InstructionData::CallIndirect {
                        opcode: Opcode::CallIndirect,
                        sig_ref,
                        args: list,
                    });
                    let ctrl = match rets.first() {
                        Some(ty) => map_type_to_cg(*ty)?,
                        None => types::I8,
                    };
                    let count = func.dfg.make_inst_results(ci, ctrl);
                    if count != rets.len() || count != results.len() {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    let cg_results: Vec<_> = func.dfg.inst_results(ci).to_vec();
                    for (rir, cg) in results.iter().zip(cg_results) {
                        vmap.insert(*rir, cg);
                    }
                }
                RirInst::GlobalValue(r, index) => {
                    let opcode = match rir.globals.get(*index as usize)? {
                        RirGlobalData::Symbol { tls: true, .. } => Opcode::TlsValue,
                        _ => Opcode::SymbolValue,
                    };
                    let ci = func.dfg.make_inst(InstructionData::UnaryGlobalValue {
                        opcode,
                        global_value: GlobalValue::from_u32(*index),
                    });
                    if func.dfg.make_inst_results(ci, types::I64) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::FuncAddr(r, target) => {
                    let func_ref = *call_refs.get(*target as usize)?;
                    let ci = func.dfg.make_inst(InstructionData::FuncAddr {
                        opcode: Opcode::FuncAddr,
                        func_ref,
                    });
                    if func.dfg.make_inst_results(ci, types::I64) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::DynamicStackAddr(r, slot) => {
                    let dynamic_stack_slot = func
                        .dynamic_stack_slots
                        .get(DynamicStackSlot::from_u32(*slot))
                        .map(|_| DynamicStackSlot::from_u32(*slot))?;
                    let ci = func.dfg.make_inst(InstructionData::DynamicStackAddr {
                        opcode: Opcode::DynamicStackAddr,
                        dynamic_stack_slot,
                    });
                    if func.dfg.make_inst_results(ci, types::I64) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::TryCall { results, target, args, normal, exception } => {
                    let func_ref = *call_refs.get(*target as usize)?;
                    let mut list = ValueList::default();
                    for v in args {
                        list.push(*vmap.get(v)?, &mut func.dfg.value_lists);
                    }
                    let normal = export_try_block(&mut func, normal, &vmap, &block_map)?;
                    let items = export_exception(&mut func, exception, &vmap, &block_map)?;
                    let sig_ref = func.dfg.ext_funcs[func_ref].signature;
                    let table = ExceptionTableData::new(sig_ref, normal, items);
                    let exception = func.dfg.exception_tables.push(table);
                    let ci = func.dfg.make_inst(InstructionData::TryCall {
                        opcode: Opcode::TryCall,
                        args: list,
                        func_ref,
                        exception,
                    });
                    if !results.is_empty() {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                }
                RirInst::TryCallIndirect {
                    results,
                    params,
                    rets,
                    call_conv,
                    callee,
                    args,
                    normal,
                    exception,
                } => {
                    let mut callee_sig = Signature::new(call_conv_from_rir(*call_conv)?);
                    for ty in params {
                        callee_sig.params.push(AbiParam::new(map_type_to_cg(*ty)?));
                    }
                    for ty in rets {
                        callee_sig.returns.push(AbiParam::new(map_type_to_cg(*ty)?));
                    }
                    let sig_ref = func.import_signature(callee_sig);
                    let mut list = ValueList::default();
                    list.push(*vmap.get(callee)?, &mut func.dfg.value_lists);
                    for arg in args {
                        list.push(*vmap.get(arg)?, &mut func.dfg.value_lists);
                    }
                    let normal = export_try_block(&mut func, normal, &vmap, &block_map)?;
                    let items = export_exception(&mut func, exception, &vmap, &block_map)?;
                    let table = ExceptionTableData::new(sig_ref, normal, items);
                    let exception = func.dfg.exception_tables.push(table);
                    let ci = func.dfg.make_inst(InstructionData::TryCallIndirect {
                        opcode: Opcode::TryCallIndirect,
                        args: list,
                        exception,
                    });
                    if !results.is_empty() {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                }
                RirInst::Jump(target, args) => {
                    let bc = make_block_call(&mut func, block_map[target.0 as usize], args, &vmap)?;
                    let ci = func
                        .dfg
                        .make_inst(InstructionData::Jump { opcode: Opcode::Jump, destination: bc });
                    func.layout.append_inst(ci, cb);
                }
                RirInst::Brif(c, t, targs, f, fargs) => {
                    let then_bc =
                        make_block_call(&mut func, block_map[t.0 as usize], targs, &vmap)?;
                    let else_bc =
                        make_block_call(&mut func, block_map[f.0 as usize], fargs, &vmap)?;
                    let ci = func.dfg.make_inst(InstructionData::Brif {
                        opcode: Opcode::Brif,
                        arg: *vmap.get(c)?,
                        blocks: [then_bc, else_bc],
                    });
                    func.layout.append_inst(ci, cb);
                }
                RirInst::Trap(code) => {
                    let ci = func.dfg.make_inst(InstructionData::Trap {
                        opcode: Opcode::Trap,
                        code: TrapCode::unwrap_user(*code),
                    });
                    func.layout.append_inst(ci, cb);
                }
                RirInst::Return(args) => {
                    let mut list = ValueList::default();
                    for v in args {
                        list.push(*vmap.get(v)?, &mut func.dfg.value_lists);
                    }
                    let ci = func.dfg.make_inst(InstructionData::MultiAry {
                        opcode: Opcode::Return,
                        args: list,
                    });
                    func.layout.append_inst(ci, cb);
                }
            }
        }
    }

    Some(func)
}

fn as_bool(builder: &mut Builder, block: RirBlock, v: RirValue) -> RirValue {
    match builder.func().ty(v) {
        RirType::Bool => v,
        ty @ (RirType::I8 | RirType::I16 | RirType::I32 | RirType::I64) => {
            let zero = builder.iconst(block, ty, 0);
            builder.icmp(block, RirIntCc::Ne, ty, v, zero)
        }
        _ => v,
    }
}

fn int_bits(ty: RirType) -> Option<u32> {
    Some(match ty {
        RirType::I8 => 8,
        RirType::I16 => 16,
        RirType::I32 => 32,
        RirType::I64 | RirType::Ptr => 64,
        RirType::I128 => 128,
        RirType::Bool => 1,
        RirType::F32 | RirType::F64 => return None,
    })
}

fn as_width(builder: &mut Builder, block: RirBlock, v: RirValue, ty: RirType, signed: bool) -> RirValue {
    let src = builder.func().ty(v);
    if src == ty {
        return v;
    }
    if src == RirType::Bool && ty.is_int() {
        return as_int(builder, block, v, ty);
    }
    if !src.is_int() || !ty.is_int() {
        return v;
    }
    let (Some(from), Some(to)) = (int_bits(src), int_bits(ty)) else {
        return v;
    };
    if from < to {
        let op = if signed { RirUnOp::Sextend } else { RirUnOp::Uextend };
        builder.unary(block, op, ty, v)
    } else if from > to {
        builder.unary(block, RirUnOp::Ireduce, ty, v)
    } else {
        v
    }
}

fn as_int(builder: &mut Builder, block: RirBlock, v: RirValue, ty: RirType) -> RirValue {
    if builder.func().ty(v) == ty {
        return v;
    }
    if builder.func().ty(v) == RirType::Bool && !ty.is_int() {
        return v;
    }
    if builder.func().ty(v) == RirType::Bool && ty.is_int() {
        let one = builder.iconst(block, ty, 1);
        let zero = builder.iconst(block, ty, 0);
        return builder.select(block, ty, v, one, zero);
    }
    as_width(builder, block, v, ty, false)
}

fn map_value(
    func: &CgFunction,
    vmap: &mut HashMap<CgValue, RirValue>,
    c: CgValue,
    r: RirValue,
) {
    vmap.insert(c, r);
    let canon = func.dfg.resolve_aliases(c);
    if canon != c {
        vmap.insert(canon, r);
    }
}

fn lookup(
    func: &CgFunction,
    vmap: &HashMap<CgValue, RirValue>,
    v: CgValue,
) -> Option<RirValue> {
    vmap.get(&func.dfg.resolve_aliases(v)).copied().or_else(|| vmap.get(&v).copied())
}

fn single_result(func: &CgFunction, inst: CgInst) -> Option<CgValue> {
    let results = func.dfg.inst_results(inst);
    if results.len() == 1 { Some(results[0]) } else { None }
}

fn typed_block_args(
    func: &CgFunction,
    bc: &BlockCall,
    vmap: &HashMap<CgValue, RirValue>,
    builder: &mut Builder,
    rb: RirBlock,
    target: RirBlock,
) -> Option<Vec<RirValue>> {
    let params = builder.func().block(target).params.clone();
    let args = bc.args(&func.dfg.value_lists).collect::<Vec<_>>();
    if args.len() != params.len() {
        return None;
    }
    let mut out = Vec::with_capacity(args.len());
    for (arg, param) in args.iter().zip(params) {
        let BlockArg::Value(v) = arg else { return None };
        let v = lookup(func, vmap, *v)?;
        let ty = builder.func().ty(param);
        out.push(if ty == RirType::Bool {
            as_bool(builder, rb, v)
        } else {
            as_int(builder, rb, v, ty)
        });
    }
    Some(out)
}

fn make_block_call(
    func: &mut CgFunction,
    target: CgBlock,
    args: &[RirValue],
    vmap: &HashMap<RirValue, CgValue>,
) -> Option<BlockCall> {
    let barg: Vec<BlockArg> =
        args.iter().map(|v| vmap.get(v).copied().map(BlockArg::Value)).collect::<Option<_>>()?;
    Some(BlockCall::new(target, barg, &mut func.dfg.value_lists))
}

fn external_name(func: &mut CgFunction, target: &RirExternalTarget) -> Option<ExternalName> {
    Some(match target {
        RirExternalTarget::User { namespace, index } => {
            let name_ref =
                func.params.ensure_user_func_name(UserExternalName::new(*namespace, *index));
            ExternalName::User(name_ref)
        }
        RirExternalTarget::LibCall(name) => ExternalName::LibCall(name.parse::<LibCall>().ok()?),
        RirExternalTarget::KnownSymbol(name) => {
            ExternalName::KnownSymbol(name.parse::<KnownSymbol>().ok()?)
        }
    })
}

fn export_try_arg(
    arg: &RirTryArg,
    vmap: &HashMap<RirValue, CgValue>,
) -> Option<BlockArg> {
    Some(match arg {
        RirTryArg::Value(v) => BlockArg::Value(*vmap.get(v)?),
        RirTryArg::Ret(i) => BlockArg::TryCallRet(*i),
        RirTryArg::Exn(i) => BlockArg::TryCallExn(*i),
    })
}

fn export_try_block(
    func: &mut CgFunction,
    call: &RirTryBlockCall,
    vmap: &HashMap<RirValue, CgValue>,
    block_map: &[CgBlock],
) -> Option<BlockCall> {
    let block = *block_map.get(call.block.0 as usize)?;
    let args = call
        .args
        .iter()
        .map(|arg| export_try_arg(arg, vmap))
        .collect::<Option<Vec<_>>>()?;
    Some(BlockCall::new(block, args, &mut func.dfg.value_lists))
}

fn export_exception(
    func: &mut CgFunction,
    exception: &[RirExceptionItem],
    vmap: &HashMap<RirValue, CgValue>,
    block_map: &[CgBlock],
) -> Option<Vec<ExceptionTableItem>> {
    exception
        .iter()
        .map(|item| {
            Some(match item {
                RirExceptionItem::Tag(tag, call) => ExceptionTableItem::Tag(
                    ExceptionTag::with_number(*tag)?,
                    export_try_block(func, call, vmap, block_map)?,
                ),
                RirExceptionItem::Default(call) => {
                    ExceptionTableItem::Default(export_try_block(func, call, vmap, block_map)?)
                }
                RirExceptionItem::Context(v) => {
                    ExceptionTableItem::Context(*vmap.get(v)?)
                }
            })
        })
        .collect()
}

fn map_external_target(func: &CgFunction, name: ExternalName) -> Option<RirExternalTarget> {
    Some(match name {
        ExternalName::User(name_ref) => {
            let name = &func.params.user_named_funcs()[name_ref];
            RirExternalTarget::User { namespace: name.namespace, index: name.index }
        }
        ExternalName::LibCall(call) => RirExternalTarget::LibCall(call.to_string()),
        ExternalName::KnownSymbol(symbol) => RirExternalTarget::KnownSymbol(symbol.to_string()),
        _ => return None,
    })
}

fn external_data(func: &CgFunction, ext_data: &ExtFuncData) -> Option<RirExternal> {
    let callee = &func.dfg.signatures[ext_data.signature];
    let params = callee
        .params
        .iter()
        .map(|p| map_type_to_rir(p.value_type))
        .collect::<Option<Vec<_>>>()?;
    let rets = callee
        .returns
        .iter()
        .map(|p| map_type_to_rir(p.value_type))
        .collect::<Option<Vec<_>>>()?;
    Some(RirExternal { target: map_external_target(func, ext_data.name.clone())?, params, rets })
}

fn external_index(
    func: &CgFunction,
    externals: &mut Vec<RirExternal>,
    ext_data: &ExtFuncData,
) -> Option<u32> {
    let external = external_data(func, ext_data)?;
    Some(match externals.iter().position(|e| *e == external) {
        Some(i) => i as u32,
        None => {
            externals.push(external);
            (externals.len() - 1) as u32
        }
    })
}

fn map_global_data(func: &CgFunction, data: &GlobalValueData) -> Option<RirGlobalData> {
    Some(match data {
        GlobalValueData::VMContext => RirGlobalData::VMContext,
        GlobalValueData::Load { base, offset, global_type, .. } => RirGlobalData::Load {
            base: base.as_u32(),
            offset: (*offset).into(),
            ty: map_type_to_rir(*global_type)?,
            flags: 0,
        },
        GlobalValueData::IAddImm { base, offset, global_type } => RirGlobalData::IAddImm {
            base: base.as_u32(),
            offset: offset.bits(),
            ty: map_type_to_rir(*global_type)?,
        },
        GlobalValueData::Symbol { name, offset, colocated, tls } => RirGlobalData::Symbol {
            target: map_external_target(func, name.clone())?,
            offset: offset.bits(),
            colocated: *colocated,
            tls: *tls,
        },
        GlobalValueData::DynScaleTargetConst { vector_type } => {
            RirGlobalData::DynScaleTargetConst { vector_type: vector_type.bits() }
        }
    })
}

fn map_try_arg(func: &CgFunction, arg: BlockArg, vmap: &HashMap<CgValue, RirValue>) -> Option<RirTryArg> {
    Some(match arg {
        BlockArg::Value(v) => RirTryArg::Value(lookup(func, vmap, v)?),
        BlockArg::TryCallRet(i) => RirTryArg::Ret(i),
        BlockArg::TryCallExn(i) => RirTryArg::Exn(i),
    })
}

fn map_try_block(
    func: &CgFunction,
    call: &BlockCall,
    vmap: &HashMap<CgValue, RirValue>,
    block_map: &HashMap<CgBlock, RirBlock>,
) -> Option<RirTryBlockCall> {
    let block = *block_map.get(&call.block(&func.dfg.value_lists))?;
    let args = call
        .args(&func.dfg.value_lists)
        .map(|arg| map_try_arg(func, arg, vmap))
        .collect::<Option<Vec<_>>>()?;
    Some(RirTryBlockCall { block, args })
}

fn map_exception(
    func: &CgFunction,
    table: &ExceptionTableData,
    vmap: &HashMap<CgValue, RirValue>,
    block_map: &HashMap<CgBlock, RirBlock>,
) -> Option<Vec<RirExceptionItem>> {
    let mut out = Vec::new();
    for item in table.items() {
        out.push(match item {
            ExceptionTableItem::Tag(tag, call) => {
                RirExceptionItem::Tag(tag.as_u32(), map_try_block(func, &call, vmap, block_map)?)
            }
            ExceptionTableItem::Default(call) => {
                RirExceptionItem::Default(map_try_block(func, &call, vmap, block_map)?)
            }
            ExceptionTableItem::Context(v) => RirExceptionItem::Context(lookup(func, vmap, v)?),
        });
    }
    Some(out)
}

/// Best-effort reason for `import` rejecting a function.  Used by
/// `RAKIT_RIR_STATS` diagnostics, never for correctness decisions.
pub fn unsupported_reason(func: &CgFunction) -> Option<String> {
    for p in func.signature.returns.iter().chain(func.signature.params.iter()) {
        if map_type_to_rir(p.value_type).is_none() {
            return Some(format!("signature type {}", p.value_type));
        }
    }
    let entry = match func.layout.entry_block() {
        Some(entry) => entry,
        None => return Some("no entry block".to_string()),
    };
    if func.signature.params.len() != func.dfg.block_params(entry).len() {
        return Some("signature/entry parameter count".to_string());
    }
    for (_, data) in func.sized_stack_slots.iter() {
        if !matches!(data.kind, StackSlotKind::ExplicitSlot) {
            return Some(format!("stack slot kind {:?}", data.kind));
        }
    }
    for (_, data) in func.dynamic_stack_slots.iter() {
        if !matches!(data.kind, StackSlotKind::ExplicitDynamicSlot) {
            return Some(format!("dynamic slot kind {:?}", data.kind));
        }
    }
    for (_, data) in func.global_values.iter() {
        if map_global_data(func, data).is_none() {
            return Some("global data".to_string());
        }
    }
    for b in func.layout.blocks() {
        for v in func.dfg.block_params(b) {
            let ty = func.dfg.value_type(*v);
            if map_type_to_rir(ty).is_none() {
                return Some(format!("block param type {ty}"));
            }
        }
    }
    for b in func.layout.blocks() {
        for inst in func.layout.block_insts(b) {
            for r in func.dfg.inst_results(inst) {
                let ty = func.dfg.value_type(*r);
                if map_type_to_rir(ty).is_none() {
                    return Some(format!("result type {ty}"));
                }
            }
            let data = func.dfg.insts[inst];
            match data {
                InstructionData::UnaryImm { opcode: Opcode::Iconst, .. } => {
                    if let Some(r) = func.dfg.inst_results(inst).first() {
                        if map_type_to_rir(func.dfg.value_type(*r)) == Some(RirType::I128) {
                            return Some("i128 constant".to_string());
                        }
                    }
                }
                InstructionData::Call { func_ref, .. } => {
                    let ext = &func.dfg.ext_funcs[func_ref];
                    let sig = &func.dfg.signatures[ext.signature];
                    if sig.params.iter().chain(sig.returns.iter()).any(|p| map_type_to_rir(p.value_type).is_none()) {
                        return Some("call signature type".to_string());
                    }
                    if !signature_supported(sig) {
                        return Some("call ABI".to_string());
                    }
                    if map_external_target(func, ext.name.clone()).is_none() {
                        return Some("call external target".to_string());
                    }
                }
                InstructionData::CallIndirect { sig_ref, .. } => {
                    let sig = &func.dfg.signatures[sig_ref];
                    if sig.params.iter().chain(sig.returns.iter()).any(|p| map_type_to_rir(p.value_type).is_none()) {
                        return Some("indirect signature type".to_string());
                    }
                    if !indirect_signature_supported(sig) {
                        return Some("indirect ABI".to_string());
                    }
                }
                InstructionData::TryCall { func_ref, exception, .. } => {
                    let ext = &func.dfg.ext_funcs[func_ref];
                    let sig = &func.dfg.signatures[ext.signature];
                    if sig.params.iter().chain(sig.returns.iter()).any(|p| map_type_to_rir(p.value_type).is_none()) {
                        return Some("try call signature type".to_string());
                    }
                    if !signature_supported(sig) {
                        return Some("try call ABI".to_string());
                    }
                    let table = &func.dfg.exception_tables[exception];
                    let table_sig = &func.dfg.signatures[table.signature()];
                    if table_sig.params.iter().chain(table_sig.returns.iter()).any(|p| map_type_to_rir(p.value_type).is_none()) {
                        return Some("try call exception signature type".to_string());
                    }
                    if !signature_supported(table_sig) {
                        return Some("try call exception ABI".to_string());
                    }
                }
                InstructionData::TryCallIndirect { exception, .. } => {
                    let table = &func.dfg.exception_tables[exception];
                    let sig = &func.dfg.signatures[table.signature()];
                    if sig.params.iter().chain(sig.returns.iter()).any(|p| map_type_to_rir(p.value_type).is_none()) {
                        return Some("try indirect signature type".to_string());
                    }
                    if !indirect_signature_supported(sig) {
                        return Some("try indirect ABI".to_string());
                    }
                }
                InstructionData::Store { opcode: Opcode::Store, args, .. }
                | InstructionData::StoreNoOffset { opcode: Opcode::Store, args, .. } => {
                    let ty = func.dfg.value_type(args[0]);
                    if map_type_to_rir(ty).is_none() {
                        return Some(format!("store type {ty}"));
                    }
                }
                InstructionData::Unary { opcode, .. }
                    if opcode != Opcode::Ineg && map_unop(opcode).is_none() =>
                {
                    return Some(format!("unary {opcode}"));
                }
                InstructionData::Unary { arg, .. } => {
                    let ty = func.dfg.value_type(arg);
                    if map_type_to_rir(ty).is_none() {
                        return Some(format!("unary operand type {ty}"));
                    }
                }
                InstructionData::Binary { opcode, .. }
                    if !matches!(opcode, Opcode::Umin | Opcode::Umax)
                        && map_binop(opcode).is_none() =>
                {
                    return Some(format!("binary {opcode}"));
                }
                InstructionData::Binary { .. } => {}
                InstructionData::Ternary { opcode, .. } if opcode != Opcode::Select => {
                    return Some(format!("ternary {opcode}"));
                }
                InstructionData::Ternary { .. } => {}
                InstructionData::MultiAry { opcode, .. } if opcode != Opcode::Return => {
                    return Some(format!("multiary {opcode}"));
                }
                InstructionData::MultiAry { .. } => {}
                InstructionData::UnaryConst { opcode, .. } => return Some(format!("const {opcode}")),
                InstructionData::BinaryImm8 { opcode, .. } => return Some(format!("imm8 {opcode}")),
                InstructionData::TernaryImm8 { opcode, .. } => return Some(format!("ternary imm8 {opcode}")),
                InstructionData::Shuffle { opcode, .. } => return Some(format!("shuffle {opcode}")),
                InstructionData::AtomicCas { opcode, .. }
                | InstructionData::AtomicRmw { opcode, .. } => return Some(format!("atomic {opcode}")),
                InstructionData::IntAddTrap { opcode, .. } => return Some(format!("add trap {opcode}")),
                InstructionData::ExceptionHandlerAddress { opcode, .. } => {
                    return Some(format!("exception address {opcode}"));
                }
                InstructionData::UnaryImm { opcode, .. } => {
                    return Some(format!("unary imm {opcode}"));
                }
                InstructionData::FuncAddr { opcode: Opcode::FuncAddr, func_ref } => {
                    if external_data(func, &func.dfg.ext_funcs[func_ref]).is_none() {
                        return Some("func addr external".to_string());
                    }
                }
                InstructionData::UnaryIeee32 { opcode: Opcode::F32const, .. }
                | InstructionData::UnaryIeee64 { opcode: Opcode::F64const, .. }
                | InstructionData::IntCompare { opcode: Opcode::Icmp, .. }
                | InstructionData::FloatCompare { opcode: Opcode::Fcmp, .. }
                | InstructionData::StackAddr { opcode: Opcode::StackAddr, .. }
                | InstructionData::Load { opcode: Opcode::Load, .. }
                | InstructionData::LoadNoOffset { opcode: Opcode::Load, .. }
                | InstructionData::UnaryGlobalValue { .. }
                | InstructionData::DynamicStackAddr { opcode: Opcode::DynamicStackAddr, .. }
                | InstructionData::CondTrap { opcode: Opcode::Trapz | Opcode::Trapnz, .. }
                | InstructionData::BranchTable { .. }
                | InstructionData::Jump { .. }
                | InstructionData::Brif { .. }
                | InstructionData::Trap { .. }
                | InstructionData::NullAry { opcode: Opcode::Nop } => {}
                _ => return Some(format!("unsupported instruction {}", data.opcode())),
            }
        }
    }
    None
}

/// Lightweight preflight used to count RIR-eligible functions for the CGU
/// budget. This deliberately mirrors the checks in `import` but does not build
/// an RIR function.
pub fn is_supported(func: &CgFunction) -> bool {
    unsupported_reason(func).is_none()
}

fn map_type_to_rir(t: CgType) -> Option<RirType> {
    Some(if t == types::I8 {
        RirType::I8
    } else if t == types::I16 {
        RirType::I16
    } else if t == types::I32 {
        RirType::I32
    } else if t == types::I64 {
        RirType::I64
    } else if t == types::I128 {
        RirType::I128
    } else if t == types::F32 {
        RirType::F32
    } else if t == types::F64 {
        RirType::F64
    } else {
        return None;
    })
}

fn map_type_to_cg(t: RirType) -> Option<CgType> {
    Some(match t {
        RirType::I8 => types::I8,
        RirType::I16 => types::I16,
        RirType::I32 => types::I32,
        RirType::I64 => types::I64,
        RirType::I128 => types::I128,
        RirType::F32 => types::F32,
        RirType::F64 => types::F64,
        RirType::Bool => types::I8,
        RirType::Ptr => types::I64,
    })
}

fn map_unop(op: Opcode) -> Option<RirUnOp> {
    Some(match op {
        Opcode::Uextend => RirUnOp::Uextend,
        Opcode::Sextend => RirUnOp::Sextend,
        Opcode::Ireduce => RirUnOp::Ireduce,
        Opcode::Bnot => RirUnOp::Bnot,
        Opcode::Bitcast => RirUnOp::Bitcast,
        Opcode::Fpromote => RirUnOp::Fpromote,
        Opcode::Fdemote => RirUnOp::Fdemote,
        Opcode::FcvtFromSint => RirUnOp::FcvtFromInt,
        Opcode::FcvtToSint => RirUnOp::FcvtToInt,
        Opcode::FcvtFromUint => RirUnOp::FcvtFromUint,
        Opcode::FcvtToUint => RirUnOp::FcvtToUint,
        _ => return None,
    })
}

fn map_unop_to_cg(op: RirUnOp) -> Option<Opcode> {
    Some(match op {
        RirUnOp::Uextend => Opcode::Uextend,
        RirUnOp::Sextend => Opcode::Sextend,
        RirUnOp::Ireduce => Opcode::Ireduce,
        RirUnOp::Bnot => Opcode::Bnot,
        RirUnOp::Bitcast => Opcode::Bitcast,
        RirUnOp::Fpromote => Opcode::Fpromote,
        RirUnOp::Fdemote => Opcode::Fdemote,
        RirUnOp::FcvtFromInt => Opcode::FcvtFromSint,
        RirUnOp::FcvtToInt => Opcode::FcvtToSint,
        RirUnOp::FcvtFromUint => Opcode::FcvtFromUint,
        RirUnOp::FcvtToUint => Opcode::FcvtToUint,
        _ => return None,
    })
}

fn map_binop(op: Opcode) -> Option<BinOp> {
    Some(match op {
        Opcode::Iadd => BinOp::Add,
        Opcode::Isub => BinOp::Sub,
        Opcode::Imul => BinOp::Mul,
        Opcode::Sdiv => BinOp::Sdiv,
        Opcode::Udiv => BinOp::Udiv,
        Opcode::Srem => BinOp::Srem,
        Opcode::Urem => BinOp::Urem,
        Opcode::Band => BinOp::And,
        Opcode::Bor => BinOp::Or,
        Opcode::Bxor => BinOp::Xor,
        Opcode::Ishl => BinOp::Shl,
        Opcode::Sshr => BinOp::Shr,
        Opcode::Ushr => BinOp::Ushr,
        Opcode::Fadd => BinOp::Fadd,
        Opcode::Fsub => BinOp::Fsub,
        Opcode::Fmul => BinOp::Fmul,
        Opcode::Fdiv => BinOp::Fdiv,
        _ => return None,
    })
}

fn map_binop_to_cg(op: BinOp) -> Option<Opcode> {
    Some(match op {
        BinOp::Add => Opcode::Iadd,
        BinOp::Sub => Opcode::Isub,
        BinOp::Mul => Opcode::Imul,
        BinOp::Sdiv => Opcode::Sdiv,
        BinOp::Udiv => Opcode::Udiv,
        BinOp::Srem => Opcode::Srem,
        BinOp::Urem => Opcode::Urem,
        BinOp::And => Opcode::Band,
        BinOp::Or => Opcode::Bor,
        BinOp::Xor => Opcode::Bxor,
        BinOp::Shl => Opcode::Ishl,
        BinOp::Shr => Opcode::Sshr,
        BinOp::Ushr => Opcode::Ushr,
        BinOp::Fadd => Opcode::Fadd,
        BinOp::Fsub => Opcode::Fsub,
        BinOp::Fmul => Opcode::Fmul,
        BinOp::Fdiv => Opcode::Fdiv,
    })
}

fn map_intcc(cc: CgIntCC) -> Option<RirIntCc> {
    Some(match cc {
        CgIntCC::Equal => RirIntCc::Eq,
        CgIntCC::NotEqual => RirIntCc::Ne,
        CgIntCC::SignedLessThan => RirIntCc::Slt,
        CgIntCC::SignedLessThanOrEqual => RirIntCc::Sle,
        CgIntCC::SignedGreaterThan => RirIntCc::Sgt,
        CgIntCC::SignedGreaterThanOrEqual => RirIntCc::Sge,
        CgIntCC::UnsignedLessThan => RirIntCc::Ult,
        CgIntCC::UnsignedLessThanOrEqual => RirIntCc::Ule,
        CgIntCC::UnsignedGreaterThan => RirIntCc::Ugt,
        CgIntCC::UnsignedGreaterThanOrEqual => RirIntCc::Uge,
    })
}

fn map_intcc_to_cg(cc: RirIntCc) -> Option<CgIntCC> {
    Some(match cc {
        RirIntCc::Eq => CgIntCC::Equal,
        RirIntCc::Ne => CgIntCC::NotEqual,
        RirIntCc::Slt => CgIntCC::SignedLessThan,
        RirIntCc::Sle => CgIntCC::SignedLessThanOrEqual,
        RirIntCc::Sgt => CgIntCC::SignedGreaterThan,
        RirIntCc::Sge => CgIntCC::SignedGreaterThanOrEqual,
        RirIntCc::Ult => CgIntCC::UnsignedLessThan,
        RirIntCc::Ule => CgIntCC::UnsignedLessThanOrEqual,
        RirIntCc::Ugt => CgIntCC::UnsignedGreaterThan,
        RirIntCc::Uge => CgIntCC::UnsignedGreaterThanOrEqual,
    })
}

fn map_floatcc(cc: CgFloatCC) -> Option<RirFloatCc> {
    Some(match cc {
        CgFloatCC::Equal => RirFloatCc::Eq,
        CgFloatCC::NotEqual => RirFloatCc::Ne,
        CgFloatCC::LessThan => RirFloatCc::Lt,
        CgFloatCC::LessThanOrEqual => RirFloatCc::Le,
        CgFloatCC::GreaterThan => RirFloatCc::Gt,
        CgFloatCC::GreaterThanOrEqual => RirFloatCc::Ge,
        _ => return None,
    })
}

fn map_floatcc_to_cg(cc: RirFloatCc) -> Option<CgFloatCC> {
    Some(match cc {
        RirFloatCc::Eq => CgFloatCC::Equal,
        RirFloatCc::Ne => CgFloatCC::NotEqual,
        RirFloatCc::Lt => CgFloatCC::LessThan,
        RirFloatCc::Le => CgFloatCC::LessThanOrEqual,
        RirFloatCc::Gt => CgFloatCC::GreaterThan,
        RirFloatCc::Ge => CgFloatCC::GreaterThanOrEqual,
    })
}

#[cfg(test)]
mod tests {
    use rakit_codegen::ir::condcodes::IntCC;
    use rakit_codegen::ir::immediates::Imm64;
    use rakit_codegen::ir::{
        Block, BlockArg, BlockCall, Function, InstructionData, MemFlagsData, Opcode, StackSlot,
        StackSlotData, StackSlotKind, Type, Value, ValueList, types,
    };
    use rakit_ir::{Val, run};

    use super::*;

    pub(crate) fn new_func(params: &[Type], rets: &[Type]) -> (Function, Block) {
        let mut func = Function::new();
        func.signature.params =
            params.iter().map(|t| rakit_codegen::ir::AbiParam::new(*t)).collect();
        func.signature.returns =
            rets.iter().map(|t| rakit_codegen::ir::AbiParam::new(*t)).collect();
        let entry = func.dfg.make_block();
        func.layout.append_block(entry);
        for t in params {
            func.dfg.append_block_param(entry, *t);
        }
        (func, entry)
    }

    fn new_block(func: &mut Function, params: &[Type]) -> Block {
        let b = func.dfg.make_block();
        func.layout.append_block(b);
        for t in params {
            func.dfg.append_block_param(b, *t);
        }
        b
    }

    pub(crate) fn iconst(func: &mut Function, b: Block, ty: Type, v: i64) -> Value {
        let inst = func
            .dfg
            .make_inst(InstructionData::UnaryImm { opcode: Opcode::Iconst, imm: Imm64::new(v) });
        assert_eq!(func.dfg.make_inst_results(inst, ty), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    fn f64const(func: &mut Function, b: Block, v: f64) -> Value {
        let inst = func.dfg.make_inst(InstructionData::UnaryIeee64 {
            opcode: Opcode::F64const,
            imm: Ieee64::with_float(v),
        });
        assert_eq!(func.dfg.make_inst_results(inst, types::F64), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    fn bin(func: &mut Function, b: Block, op: Opcode, ty: Type, x: Value, y: Value) -> Value {
        let inst = func.dfg.make_inst(InstructionData::Binary { opcode: op, args: [x, y] });
        assert_eq!(func.dfg.make_inst_results(inst, ty), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    fn unary(func: &mut Function, b: Block, op: Opcode, ty: Type, x: Value) -> Value {
        let inst = func.dfg.make_inst(InstructionData::Unary { opcode: op, arg: x });
        assert_eq!(func.dfg.make_inst_results(inst, ty), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    fn icmp(func: &mut Function, b: Block, cc: IntCC, x: Value, y: Value) -> Value {
        let inst = func.dfg.make_inst(InstructionData::IntCompare {
            opcode: Opcode::Icmp,
            args: [x, y],
            cond: cc,
        });
        assert_eq!(func.dfg.make_inst_results(inst, types::I8), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    pub(crate) fn ret(func: &mut Function, b: Block, vals: &[Value]) {
        let mut list = ValueList::default();
        for v in vals {
            list.push(*v, &mut func.dfg.value_lists);
        }
        let inst =
            func.dfg.make_inst(InstructionData::MultiAry { opcode: Opcode::Return, args: list });
        func.layout.append_inst(inst, b);
    }

    fn jump(func: &mut Function, b: Block, target: Block, args: &[Value]) {
        let bc = BlockCall::new(
            target,
            args.iter().copied().map(BlockArg::Value),
            &mut func.dfg.value_lists,
        );
        let inst =
            func.dfg.make_inst(InstructionData::Jump { opcode: Opcode::Jump, destination: bc });
        func.layout.append_inst(inst, b);
    }

    fn brif(
        func: &mut Function,
        b: Block,
        cond: Value,
        then_b: Block,
        then_args: &[Value],
        else_b: Block,
        else_args: &[Value],
    ) {
        let tb = BlockCall::new(
            then_b,
            then_args.iter().copied().map(BlockArg::Value),
            &mut func.dfg.value_lists,
        );
        let eb = BlockCall::new(
            else_b,
            else_args.iter().copied().map(BlockArg::Value),
            &mut func.dfg.value_lists,
        );
        let inst = func.dfg.make_inst(InstructionData::Brif {
            opcode: Opcode::Brif,
            arg: cond,
            blocks: [tb, eb],
        });
        func.layout.append_inst(inst, b);
    }

    fn stack_slot(func: &mut Function, size: u32, align_shift: u8) -> StackSlot {
        func.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            size,
            align_shift,
        ))
    }

    fn stack_addr(func: &mut Function, b: Block, slot: StackSlot, offset: i32) -> Value {
        let inst = func.dfg.make_inst(InstructionData::StackAddr {
            opcode: Opcode::StackAddr,
            stack_slot: slot,
            offset: offset.into(),
        });
        assert_eq!(func.dfg.make_inst_results(inst, types::I64), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    fn store(func: &mut Function, b: Block, ty: Type, addr: Value, value: Value, offset: i32) {
        assert_eq!(func.dfg.value_type(value), ty);
        let flags = func.dfg.mem_flags.insert(MemFlagsData::new()).unwrap();
        let inst = func.dfg.make_inst(InstructionData::Store {
            opcode: Opcode::Store,
            flags,
            args: [value, addr],
            offset: offset.into(),
        });
        func.layout.append_inst(inst, b);
    }

    fn load(func: &mut Function, b: Block, ty: Type, addr: Value, offset: i32) -> Value {
        let flags = func.dfg.mem_flags.insert(MemFlagsData::new()).unwrap();
        let inst = func.dfg.make_inst(InstructionData::Load {
            opcode: Opcode::Load,
            flags,
            arg: addr,
            offset: offset.into(),
        });
        assert_eq!(func.dfg.make_inst_results(inst, ty), 1);
        func.layout.append_inst(inst, b);
        func.dfg.inst_results(inst)[0]
    }

    #[test]
    fn memory_roundtrip() {
        let (mut f, e) = new_func(&[types::I64], &[types::I64]);
        let x = f.dfg.block_params(e)[0];
        let slot = stack_slot(&mut f, 16, 3);
        let addr = stack_addr(&mut f, e, slot, 0);
        store(&mut f, e, types::I64, addr, x, 0);
        let y = load(&mut f, e, types::I64, addr, 0);
        ret(&mut f, e, &[y]);

        let mut rir = import(&f).expect("import");
        assert_eq!(run(&rir, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
        let _ = rakit_ir::optimize(&mut rir);
        let out = export(&rir, &f.signature).expect("export");
        let rir2 = import(&out).expect("re-import");
        assert_eq!(run(&rir2, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
    }

    #[test]
    fn memory_flags_roundtrip() {
        let (mut f, e) = new_func(&[types::I64], &[types::I64]);
        let x = f.dfg.block_params(e)[0];
        let slot = stack_slot(&mut f, 16, 3);
        let addr = stack_addr(&mut f, e, slot, 0);
        let flags_data = MemFlagsData::trusted().with_readonly().with_can_move();
        let flags = f.dfg.mem_flags.insert(flags_data).unwrap();
        let store = f.dfg.make_inst(InstructionData::Store {
            opcode: Opcode::Store,
            flags,
            args: [x, addr],
            offset: 0.into(),
        });
        f.layout.append_inst(store, e);
        let load = f.dfg.make_inst(InstructionData::Load {
            opcode: Opcode::Load,
            flags,
            arg: addr,
            offset: 0.into(),
        });
        assert_eq!(f.dfg.make_inst_results(load, types::I64), 1);
        f.layout.append_inst(load, e);
        let y = f.dfg.inst_results(load)[0];
        ret(&mut f, e, &[y]);

        let rir = import(&f).expect("import");
        let expected = RirMemFlags(flags_data.bits());
        assert!(rir.blocks.iter().flat_map(|b| &b.insts).any(|inst| {
            matches!(inst, RirInst::Store(.., flags) if *flags == expected)
        }));
        assert!(rir.blocks.iter().flat_map(|b| &b.insts).any(|inst| {
            matches!(inst, RirInst::Load(.., flags) if *flags == expected)
        }));

        let out = export(&rir, &f.signature).expect("export");
        let bits: Vec<u16> = out
            .layout
            .blocks()
            .flat_map(|b| out.layout.block_insts(b))
            .filter_map(|inst| match out.dfg.insts[inst] {
                InstructionData::Store { flags, .. } | InstructionData::Load { flags, .. } => {
                    Some(out.dfg.mem_flags[flags].bits())
                }
                _ => None,
            })
            .collect();
        assert_eq!(bits, vec![expected.0, expected.0]);
    }

    #[test]
    fn import_export_add() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let two = iconst(&mut f, e, types::I64, 2);
        let three = iconst(&mut f, e, types::I64, 3);
        let sum = bin(&mut f, e, Opcode::Iadd, types::I64, two, three);
        ret(&mut f, e, &[sum]);

        let mut rir = import(&f).expect("import");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(5)]);
        assert!(rakit_ir::optimize(&mut rir));
        let out = export(&rir, &f.signature).expect("export");
        let rir2 = import(&out).expect("re-import");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(5)]);
    }

    #[test]
    fn branch_folding_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let then_b = new_block(&mut f, &[]);
        let else_b = new_block(&mut f, &[]);
        let a = iconst(&mut f, e, types::I64, 5);
        let b = iconst(&mut f, e, types::I64, 3);
        let cond = icmp(&mut f, e, IntCC::SignedGreaterThan, a, b);
        brif(&mut f, e, cond, then_b, &[], else_b, &[]);
        let one = iconst(&mut f, then_b, types::I64, 1);
        ret(&mut f, then_b, &[one]);
        let two = iconst(&mut f, else_b, types::I64, 2);
        ret(&mut f, else_b, &[two]);

        let mut rir = import(&f).expect("import");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(1)]);
        assert!(rakit_ir::optimize(&mut rir));
        let out = export(&rir, &f.signature).expect("export");
        let rir2 = import(&out).expect("re-import");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(1)]);
    }

    #[test]
    fn block_param_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let t = new_block(&mut f, &[types::I64]);
        let one = iconst(&mut f, e, types::I64, 1);
        jump(&mut f, e, t, &[one]);
        let p = f.dfg.block_params(t)[0];
        ret(&mut f, t, &[p]);

        let rir = import(&f).expect("import");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(1)]);
        let out = export(&rir, &f.signature).expect("export");
        let rir2 = import(&out).expect("re-import");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(1)]);
    }

    #[test]
    fn bool_truthy_integer_use_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I8]);
        let a = iconst(&mut f, e, types::I64, 5);
        let b = iconst(&mut f, e, types::I64, 3);
        let cond = icmp(&mut f, e, IntCC::SignedGreaterThan, a, b);
        let one = iconst(&mut f, e, types::I8, 1);
        let sum = bin(&mut f, e, Opcode::Iadd, types::I8, cond, one);
        ret(&mut f, e, &[sum]);

        let mut rir = import(&f).expect("import bool truthy");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(2)]);
        assert!(rakit_ir::optimize(&mut rir));
        let out = export(&rir, &f.signature).expect("export bool truthy");
        let rir2 = import(&out).expect("re-import bool truthy");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(2)]);
    }

    #[test]
    fn comparison_to_float_cast_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::F64]);
        let a = iconst(&mut f, e, types::I64, 5);
        let b = iconst(&mut f, e, types::I64, 3);
        let cond = icmp(&mut f, e, IntCC::SignedGreaterThan, a, b);
        let v = unary(&mut f, e, Opcode::FcvtFromUint, types::F64, cond);
        ret(&mut f, e, &[v]);

        let rir = import(&f).expect("import comparison-to-float");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::F64(1.0)]);
        let out = export(&rir, &f.signature).expect("export comparison-to-float");
        let rir2 = import(&out).expect("re-import comparison-to-float");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::F64(1.0)]);
    }

    #[test]
    fn signed_extension_and_compare_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let x = iconst(&mut f, e, types::I8, -1);
        let ext = unary(&mut f, e, Opcode::Sextend, types::I64, x);
        ret(&mut f, e, &[ext]);
        let rir = import(&f).expect("import sextend");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(-1)]);
        let out = export(&rir, &f.signature).expect("export sextend");
        let rir2 = import(&out).expect("re-import sextend");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(-1)]);

        let (mut f, e) = new_func(&[], &[types::I8]);
        let x = iconst(&mut f, e, types::I8, -1);
        let zero = iconst(&mut f, e, types::I8, 0);
        let cond = icmp(&mut f, e, IntCC::SignedLessThan, x, zero);
        ret(&mut f, e, &[cond]);
        let rir = import(&f).expect("import signed compare");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(1)]);
        let out = export(&rir, &f.signature).expect("export signed compare");
        let rir2 = import(&out).expect("re-import signed compare");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(1)]);
    }

    #[test]
    fn shift_amount_width_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let x = iconst(&mut f, e, types::I64, 8);
        let y = iconst(&mut f, e, types::I32, 3);
        let shifted = bin(&mut f, e, Opcode::Ishl, types::I64, x, y);
        ret(&mut f, e, &[shifted]);

        let rir = import(&f).expect("import shift width");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(64)]);
        let out = export(&rir, &f.signature).expect("export shift width");
        let rir2 = import(&out).expect("re-import shift width");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(64)]);
    }

    #[test]
    fn branch_arg_width_coercion_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let target = new_block(&mut f, &[types::I64]);
        let small = iconst(&mut f, e, types::I32, 7);
        jump(&mut f, e, target, &[small]);
        let param = f.dfg.block_params(target)[0];
        ret(&mut f, target, &[param]);

        let rir = import(&f).expect("import branch width");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::I(7)]);
        let out = export(&rir, &f.signature).expect("export branch width");
        let rir2 = import(&out).expect("re-import branch width");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::I(7)]);
    }

    #[test]
    fn float_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::F64]);
        let x = f64const(&mut f, e, 1.5);
        let y = f64const(&mut f, e, 2.5);
        let sum = bin(&mut f, e, Opcode::Fadd, types::F64, x, y);
        ret(&mut f, e, &[sum]);

        let mut rir = import(&f).expect("import");
        assert_eq!(run(&rir, &[]).unwrap(), vec![Val::F64(4.0)]);
        assert!(rakit_ir::optimize(&mut rir));
        let out = export(&rir, &f.signature).expect("export");
        let rir2 = import(&out).expect("re-import");
        assert_eq!(run(&rir2, &[]).unwrap(), vec![Val::F64(4.0)]);
    }


    #[test]
    fn global_value_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let name_ref = f.params.ensure_user_func_name(UserExternalName::new(0, 41));
        let global = f.create_global_value(GlobalValueData::Symbol {
            name: ExternalName::User(name_ref),
            offset: Imm64::new(4),
            colocated: false,
            tls: false,
        });
        let inst = f.dfg.make_inst(InstructionData::UnaryGlobalValue {
            opcode: Opcode::SymbolValue,
            global_value: global,
        });
        assert_eq!(f.dfg.make_inst_results(inst, types::I64), 1);
        f.layout.append_inst(inst, e);
        let value = f.dfg.inst_results(inst)[0];
        ret(&mut f, e, &[value]);

        let rir = import(&f).expect("import global");
        assert_eq!(rir.globals.len(), 1);
        assert!(rir.blocks[0].insts.iter().any(|i| matches!(i, RirInst::GlobalValue(..))));
        let out = export(&rir, &f.signature).expect("export global");
        let rir2 = import(&out).expect("re-import global");
        assert_eq!(rir2.globals, rir.globals);
        assert_eq!(rir2.externals, rir.externals);
    }

    #[test]
    fn dynamic_stack_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let scale = f.create_global_value(GlobalValueData::VMContext);
        let dyn_ty = f
            .dfg
            .dynamic_types
            .push(DynamicTypeData::new(types::I8X16, scale));
        let slot = f.create_dynamic_stack_slot(DynamicStackSlotData::new(
            StackSlotKind::ExplicitDynamicSlot,
            dyn_ty,
        ));
        let inst = f.dfg.make_inst(InstructionData::DynamicStackAddr {
            opcode: Opcode::DynamicStackAddr,
            dynamic_stack_slot: slot,
        });
        assert_eq!(f.dfg.make_inst_results(inst, types::I64), 1);
        f.layout.append_inst(inst, e);
        let value = f.dfg.inst_results(inst)[0];
        ret(&mut f, e, &[value]);

        let rir = import(&f).expect("import dynamic stack");
        assert_eq!(rir.dynamic_stack_slots.len(), 1);
        assert_eq!(rir.dynamic_types.len(), 1);
        let out = export(&rir, &f.signature).expect("export dynamic stack");
        let rir2 = import(&out).expect("re-import dynamic stack");
        assert_eq!(rir2.dynamic_stack_slots, rir.dynamic_stack_slots);
        assert_eq!(rir2.dynamic_types, rir.dynamic_types);
    }

    #[test]
    fn try_call_roundtrip() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let mut callee = Signature::new(CallConv::SystemV);
        callee.returns.push(AbiParam::new(types::I64));
        let sig_ref = f.import_signature(callee);
        let name_ref = f.params.ensure_user_func_name(UserExternalName::new(0, 42));
        let func_ref = f.import_function(ExtFuncData {
            name: ExternalName::User(name_ref),
            signature: sig_ref,
            colocated: false,
            patchable: false,
        });
        let normal = new_block(&mut f, &[types::I64]);
        let exception_block = new_block(&mut f, &[types::I64]);
        let normal_call = BlockCall::new(
            normal,
            [BlockArg::TryCallRet(0)],
            &mut f.dfg.value_lists,
        );
        let exception_call = BlockCall::new(
            exception_block,
            [BlockArg::TryCallExn(0)],
            &mut f.dfg.value_lists,
        );
        let table = ExceptionTableData::new(
            sig_ref,
            normal_call,
            [ExceptionTableItem::Default(exception_call)],
        );
        let exception_table = f.dfg.exception_tables.push(table);
        let inst = f.dfg.make_inst(InstructionData::TryCall {
            opcode: Opcode::TryCall,
            args: ValueList::default(),
            func_ref,
            exception: exception_table,
        });
        assert_eq!(f.dfg.make_inst_results(inst, types::I64), 0);
        f.layout.append_inst(inst, e);
        let normal_param = f.dfg.block_params(normal)[0];
        ret(&mut f, normal, &[normal_param]);
        let exception_param = f.dfg.block_params(exception_block)[0];
        ret(&mut f, exception_block, &[exception_param]);

        assert!(is_supported(&f), "{:?}", unsupported_reason(&f));
        let rir = import(&f).expect("import try call");
        assert!(rir.blocks[0].insts.iter().any(|i| matches!(i, RirInst::TryCall { .. })));
        let out = export(&rir, &f.signature).expect("export try call");
        let rir2 = import(&out).expect("re-import try call");
        assert_eq!(rir2.externals, rir.externals);
        assert!(rir2.blocks[0].insts.iter().any(|i| matches!(i, RirInst::TryCall { .. })));
    }

    #[test]
    fn i128_folded_zero_export_is_legal() {
        let (mut f, e) = new_func(&[types::I128], &[types::I128]);
        let x = f.dfg.block_params(e)[0];
        let v = bin(&mut f, e, Opcode::Bxor, types::I128, x, x);
        ret(&mut f, e, &[v]);

        let mut rir = import(&f).expect("import i128 xor");
        assert!(rakit_ir::optimize(&mut rir));
        let out = export(&rir, &f.signature).expect("export folded i128 zero");
        let flags = rakit_codegen::settings::Flags::new(rakit_codegen::settings::builder());
        rakit_codegen::verifier::verify_function(&out, &flags)
            .expect("exported i128 zero must verify");
        assert_eq!(run(&rir, &[Val::I128(7)]).unwrap(), vec![Val::I128(0)]);
    }

    #[test]
    fn preflight_accepts_supported_binary_select_and_return() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let one = iconst(&mut f, e, types::I64, 1);
        let two = iconst(&mut f, e, types::I64, 2);
        let sum = bin(&mut f, e, Opcode::Iadd, types::I64, one, two);
        let cond = icmp(&mut f, e, IntCC::UnsignedLessThan, one, two);
        let select = f.dfg.make_inst(InstructionData::Ternary {
            opcode: Opcode::Select,
            args: [cond, sum, one],
        });
        assert_eq!(f.dfg.make_inst_results(select, types::I64), 1);
        f.layout.append_inst(select, e);
        let selected = f.dfg.inst_results(select)[0];
        ret(&mut f, e, &[selected]);

        assert!(unsupported_reason(&f).is_none(), "{:?}", unsupported_reason(&f));
        assert!(is_supported(&f));
        assert!(import(&f).is_some(), "preflight must agree with import");
    }

    #[test]
    fn unsupported_opcode_is_rejected() {
        let (mut f, e) = new_func(&[types::F64], &[types::F64]);
        let p = f.dfg.block_params(e)[0];
        let inst = f.dfg.make_inst(InstructionData::Unary { opcode: Opcode::Fneg, arg: p });
        assert_eq!(f.dfg.make_inst_results(inst, types::F64), 1);
        f.layout.append_inst(inst, e);
        let r = f.dfg.inst_results(inst)[0];
        ret(&mut f, e, &[r]);
        assert!(import(&f).is_none());
    }
}

#[cfg(test)]
mod call_tests {
    use rakit_codegen::ir::{
        AbiParam, ExtFuncData, ExternalName, InstructionData, Opcode, Signature, UserExternalName,
        ValueList, types,
    };

    use super::*;
    use super::tests::{iconst, ret};

    fn new_func(params: &[CgType], rets: &[CgType]) -> (CgFunction, CgBlock) {
        let mut func = CgFunction::new();
        func.signature.params = params.iter().map(|t| AbiParam::new(*t)).collect();
        func.signature.returns = rets.iter().map(|t| AbiParam::new(*t)).collect();
        let entry = func.dfg.make_block();
        func.layout.append_block(entry);
        for t in params {
            func.dfg.append_block_param(entry, *t);
        }
        (func, entry)
    }

    #[test]
    fn import_export_call() {
        let (mut func, entry) = new_func(&[types::I64], &[types::I64]);
        let param = func.dfg.block_params(entry)[0];

        let mut callee = Signature::new(CallConv::SystemV);
        callee.params.push(AbiParam::new(types::I64));
        callee.returns.push(AbiParam::new(types::I64));
        let sig_ref = func.import_signature(callee);
        let name_ref = func.params.ensure_user_func_name(UserExternalName::new(0, 7));
        let func_ref = func.import_function(ExtFuncData {
            name: ExternalName::User(name_ref),
            signature: sig_ref,
            colocated: false,
            patchable: false,
        });
        let mut args = ValueList::default();
        args.push(param, &mut func.dfg.value_lists);
        let call =
            func.dfg.make_inst(InstructionData::Call { opcode: Opcode::Call, func_ref, args });
        assert_eq!(func.dfg.make_inst_results(call, types::I64), 1);
        func.layout.append_inst(call, entry);
        let result = func.dfg.first_result(call);
        let mut ret_args = ValueList::default();
        ret_args.push(result, &mut func.dfg.value_lists);
        let ret = func
            .dfg
            .make_inst(InstructionData::MultiAry { opcode: Opcode::Return, args: ret_args });
        func.layout.append_inst(ret, entry);

        let rir = import(&func).expect("import call");
        assert_eq!(rir.externals.len(), 1);
        assert!(rir.blocks[0].insts.iter().any(|i| matches!(i, RirInst::Call(..))));

        let out = export(&rir, &func.signature).expect("export call");
        let rir2 = import(&out).expect("re-import call");
        assert_eq!(rir2.externals, rir.externals);
    }

    #[test]
    fn import_export_multi_result_call() {
        let (mut func, entry) = new_func(&[types::I64], &[types::I64, types::I64]);
        let param = func.dfg.block_params(entry)[0];

        let mut callee = Signature::new(CallConv::SystemV);
        callee.params.push(AbiParam::new(types::I64));
        callee.returns.push(AbiParam::new(types::I64));
        callee.returns.push(AbiParam::new(types::I64));
        let sig_ref = func.import_signature(callee);
        let name_ref = func.params.ensure_user_func_name(UserExternalName::new(0, 9));
        let func_ref = func.import_function(ExtFuncData {
            name: ExternalName::User(name_ref),
            signature: sig_ref,
            colocated: false,
            patchable: false,
        });
        let mut args = ValueList::default();
        args.push(param, &mut func.dfg.value_lists);
        let call =
            func.dfg.make_inst(InstructionData::Call { opcode: Opcode::Call, func_ref, args });
        assert_eq!(func.dfg.make_inst_results(call, types::I64), 2);
        func.layout.append_inst(call, entry);
        let results: Vec<_> = func.dfg.inst_results(call).to_vec();
        let mut ret_args = ValueList::default();
        for result in &results {
            ret_args.push(*result, &mut func.dfg.value_lists);
        }
        let ret = func
            .dfg
            .make_inst(InstructionData::MultiAry { opcode: Opcode::Return, args: ret_args });
        func.layout.append_inst(ret, entry);

        let rir = import(&func).expect("import multi-result call");
        assert_eq!(rir.externals.len(), 1);
        assert_eq!(rir.externals[0].rets.len(), 2);
        assert!(rir.blocks[0].insts.iter().any(|i| matches!(
            i,
            RirInst::Call(results, _, _) if results.len() == 2
        )));

        let out = export(&rir, &func.signature).expect("export multi-result call");
        let rir2 = import(&out).expect("re-import multi-result call");
        assert_eq!(rir2.externals, rir.externals);
    }

    #[test]
    fn func_addr_with_vector_signature_is_rejected() {
        let (mut func, entry) = new_func(&[], &[types::I64]);
        let mut callee = Signature::new(CallConv::SystemV);
        callee.params.push(AbiParam::new(types::I8X16));
        callee.returns.push(AbiParam::new(types::I8X16));
        let sig_ref = func.import_signature(callee);
        let name_ref = func.params.ensure_user_func_name(UserExternalName::new(0, 77));
        let func_ref = func.import_function(ExtFuncData {
            name: ExternalName::User(name_ref),
            signature: sig_ref,
            colocated: false,
            patchable: false,
        });
        let inst = func.dfg.make_inst(InstructionData::FuncAddr {
            opcode: Opcode::FuncAddr,
            func_ref,
        });
        assert_eq!(func.dfg.make_inst_results(inst, types::I64), 1);
        func.layout.append_inst(inst, entry);
        let addr = func.dfg.inst_results(inst)[0];
        ret(&mut func, entry, &[addr]);

        assert!(!is_supported(&func), "vector FuncAddr must not count as eligible");
        assert!(unsupported_reason(&func).is_some());
        assert!(import(&func).is_none(), "import must agree with preflight");
    }

    #[test]
    fn import_export_indirect_call() {
        let (mut func, entry) = new_func(&[types::I64, types::I64], &[types::I64]);
        let params = func.dfg.block_params(entry).to_vec();

        let mut callee = Signature::new(CallConv::SystemV);
        callee.params.push(AbiParam::new(types::I64));
        callee.returns.push(AbiParam::new(types::I64));
        let sig_ref = func.import_signature(callee);

        let mut args = ValueList::default();
        args.push(params[0], &mut func.dfg.value_lists);
        args.push(params[1], &mut func.dfg.value_lists);
        let call = func.dfg.make_inst(InstructionData::CallIndirect {
            opcode: Opcode::CallIndirect,
            sig_ref,
            args,
        });
        assert_eq!(func.dfg.make_inst_results(call, types::I64), 1);
        func.layout.append_inst(call, entry);
        let result = func.dfg.first_result(call);
        let mut ret_args = ValueList::default();
        ret_args.push(result, &mut func.dfg.value_lists);
        let ret = func
            .dfg
            .make_inst(InstructionData::MultiAry { opcode: Opcode::Return, args: ret_args });
        func.layout.append_inst(ret, entry);

        let rir = import(&func).expect("import indirect call");
        assert!(rir.blocks[0].insts.iter().any(|i| matches!(i, RirInst::CallIndirect { .. })));
        let out = export(&rir, &func.signature).expect("export indirect call");
        let rir2 = import(&out).expect("re-import indirect call");
        assert!(rir2.blocks[0].insts.iter().any(|i| matches!(i, RirInst::CallIndirect { .. })));
    }

    #[test]
    fn import_export_indirect_call_preserves_non_systemv_abi() {
        let (mut func, entry) = new_func(&[types::I64, types::I64], &[types::I64]);
        let params = func.dfg.block_params(entry).to_vec();

        let mut callee = Signature::new(CallConv::Fast);
        callee.params.push(AbiParam::new(types::I64));
        callee.returns.push(AbiParam::new(types::I64));
        let sig_ref = func.import_signature(callee);

        let mut args = ValueList::default();
        args.push(params[0], &mut func.dfg.value_lists);
        args.push(params[1], &mut func.dfg.value_lists);
        let call = func.dfg.make_inst(InstructionData::CallIndirect {
            opcode: Opcode::CallIndirect,
            sig_ref,
            args,
        });
        assert_eq!(func.dfg.make_inst_results(call, types::I64), 1);
        func.layout.append_inst(call, entry);
        let result = func.dfg.first_result(call);
        ret(&mut func, entry, &[result]);

        let rir = import(&func).expect("import fast indirect call");
        assert!(rir.blocks[0].insts.iter().any(|i| matches!(
            i,
            RirInst::CallIndirect { call_conv, .. } if *call_conv == call_conv_to_rir(CallConv::Fast)
        )));
        let out = export(&rir, &func.signature).expect("export fast indirect call");
        let rir2 = import(&out).expect("re-import fast indirect call");
        assert!(rir2.blocks[0].insts.iter().any(|i| matches!(
            i,
            RirInst::CallIndirect { call_conv, .. } if *call_conv == call_conv_to_rir(CallConv::Fast)
        )));
    }

    #[test]
    fn fence_is_not_rir_eligible() {
        let (mut f, e) = new_func(&[], &[]);
        let fence = f.dfg.make_inst(InstructionData::NullAry { opcode: Opcode::Fence });
        f.layout.append_inst(fence, e);
        ret(&mut f, e, &[]);
        assert!(import(&f).is_none(), "import must reject a fence");
        assert!(!is_supported(&f), "preflight must reject a fence");
        assert!(
            unsupported_reason(&f).is_some(),
            "preflight must give a reason for a fence"
        );
    }

    #[test]
    fn rejects_non_systemv_external_signature() {
        let (mut f, e) = new_func(&[], &[types::I64]);
        let mut callee = Signature::new(CallConv::WindowsFastcall);
        callee.params.push(AbiParam::new(types::I64));
        callee.returns.push(AbiParam::new(types::I64));
        let sig_ref = f.import_signature(callee);
        let name_ref = f.params.ensure_user_func_name(UserExternalName::new(0, 3));
        let func_ref = f.import_function(ExtFuncData {
            name: ExternalName::User(name_ref),
            signature: sig_ref,
            colocated: false,
            patchable: false,
        });
        let one = iconst(&mut f, e, types::I64, 1);
        let mut args = ValueList::default();
        args.push(one, &mut f.dfg.value_lists);
        let call =
            f.dfg.make_inst(InstructionData::Call { opcode: Opcode::Call, func_ref, args });
        assert_eq!(f.dfg.make_inst_results(call, types::I64), 1);
        f.layout.append_inst(call, e);
        let r = f.dfg.first_result(call);
        ret(&mut f, e, &[r]);
        assert!(import(&f).is_none(), "win64 signature must not be round-tripped as SysV");
    }
}
