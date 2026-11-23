//! CLIF <-> RIR bridge for a conservative scalar subset.
//!
//! The bridge is intentionally partial: it accepts functions that use only
//! scalar constants, integer/float arithmetic, comparisons, `select`, block
//! parameters, `jump`/`brif` and `return`, and refuses everything else (calls,
//! memory, stack slots, globals, exception edges, multi-result instructions).
//! Refusing is always safe: the caller leaves the function untouched.

use std::collections::HashMap;

use rakit_codegen::ir::condcodes::{FloatCC as ClifFloatCC, IntCC as ClifIntCC};
use rakit_codegen::ir::immediates::{Ieee32, Ieee64, Imm64};
use rakit_codegen::ir::{
    AbiParam, Block as ClifBlock, BlockArg, BlockCall, ExtFuncData, ExternalName,
    Function as ClifFunction, Inst as ClifInst, InstructionData, KnownSymbol, LibCall,
    MemFlagsData, Opcode, Signature, StackSlot as ClifStackSlot,
    StackSlotData as ClifStackSlotData, StackSlotKind, Type as ClifType, UserExternalName,
    Value as ClifValue, ValueList, types,
};
use rakit_codegen::isa::CallConv;
use rakit_ir::{
    BinOp, Block as RirBlock, Builder, External as RirExternal,
    ExternalTarget as RirExternalTarget, FloatCc as RirFloatCc, Function as RirFunction,
    Inst as RirInst, IntCc as RirIntCc, StackSlot as RirStackSlot, Type as RirType,
    Value as RirValue,
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

/// Import a CLIF function into RIR, or `None` if it uses anything outside the
/// supported scalar subset.
pub fn import(func: &ClifFunction) -> Option<RirFunction> {
    if func.global_values.len() != 0
        || func.dynamic_stack_slots.len() != 0
        || func.stack_limit.is_some()
    {
        return None;
    }

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

    let mut builder = Builder::new(ret, entry_params);
    let mut externals: Vec<RirExternal> = Vec::new();
    let mut slot_map: HashMap<ClifStackSlot, RirStackSlot> = HashMap::new();
    for (cs, data) in func.sized_stack_slots.iter() {
        if !matches!(data.kind, StackSlotKind::ExplicitSlot) {
            return None;
        }
        let align = 1u32.checked_shl(data.align_shift as u32).unwrap_or(1);
        let rs = builder.add_stack_slot(data.size, align);
        slot_map.insert(cs, rs);
    }
    let mut block_map: HashMap<ClifBlock, RirBlock> = HashMap::new();
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

    let mut vmap: HashMap<ClifValue, RirValue> = HashMap::new();
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
        let rb = *block_map.get(&b)?;
        for inst in func.layout.block_insts(b) {
            let data = func.dfg.insts[inst];
            match data {
                InstructionData::UnaryImm { opcode: Opcode::Iconst, imm } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
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
                InstructionData::Binary { opcode, args } => {
                    let op = map_binop(opcode)?;
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let x = lookup(func, &vmap, args[0])?;
                    let y = lookup(func, &vmap, args[1])?;
                    let v = builder.bin(rb, op, ty, x, y);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::IntCompare { opcode: Opcode::Icmp, args, cond } => {
                    let cc = map_intcc(cond)?;
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    let x = lookup(func, &vmap, args[0])?;
                    let y = lookup(func, &vmap, args[1])?;
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
                    let x = lookup(func, &vmap, args[1])?;
                    let y = lookup(func, &vmap, args[2])?;
                    let v = builder.select(rb, ty, c, x, y);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::StackAddr { opcode: Opcode::StackAddr, stack_slot, offset } => {
                    let r = single_result(func, inst)?;
                    let slot = *slot_map.get(&stack_slot)?;
                    let v = builder.stack_addr(rb, slot, offset.into());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Load { opcode: Opcode::Load, arg, offset, .. } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let addr = lookup(func, &vmap, arg)?;
                    let v = builder.load(rb, ty, addr, offset.into());
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::LoadNoOffset { opcode: Opcode::Load, arg, .. } => {
                    let r = single_result(func, inst)?;
                    let ty = map_type_to_rir(func.dfg.value_type(r))?;
                    let addr = lookup(func, &vmap, arg)?;
                    let v = builder.load(rb, ty, addr, 0);
                    map_value(func, &mut vmap, r, v);
                }
                InstructionData::Store { opcode: Opcode::Store, args, offset, .. } => {
                    let value = lookup(func, &vmap, args[0])?;
                    let addr = lookup(func, &vmap, args[1])?;
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    builder.store(rb, ty, addr, offset.into(), value);
                }
                InstructionData::StoreNoOffset { opcode: Opcode::Store, args, .. } => {
                    let value = lookup(func, &vmap, args[0])?;
                    let addr = lookup(func, &vmap, args[1])?;
                    let ty = map_type_to_rir(func.dfg.value_type(args[0]))?;
                    builder.store(rb, ty, addr, 0, value);
                }
                InstructionData::Call { func_ref, args, .. } => {
                    let ext_data = &func.dfg.ext_funcs[func_ref];
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
                    let external = RirExternal { target, params, rets };
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
                        .map(|v| lookup(func, &vmap, *v))
                        .collect::<Option<Vec<_>>>()?;
                    let results: Vec<_> = func.dfg.inst_results(inst).to_vec();
                    let rets = results
                        .iter()
                        .map(|r| map_type_to_rir(func.dfg.value_type(*r)))
                        .collect::<Option<Vec<_>>>()?;
                    let rir_results = builder.call_multi(rb, target, args, rets);
                    for (clif, rir) in results.iter().zip(rir_results) {
                        map_value(func, &mut vmap, *clif, rir);
                    }
                }
                InstructionData::CallIndirect { sig_ref, args, .. } => {
                    let callee = args.as_slice(&func.dfg.value_lists)[0];
                    let callee = lookup(func, &vmap, callee)?;
                    let call_args = args.as_slice(&func.dfg.value_lists)[1..]
                        .iter()
                        .map(|v| lookup(func, &vmap, *v))
                        .collect::<Option<Vec<_>>>()?;
                    let callee_sig = &func.dfg.signatures[sig_ref];
                    let params = callee_sig
                        .params
                        .iter()
                        .map(|p| map_type_to_rir(p.value_type))
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
                    for (clif, rir) in results.iter().zip(rir_results) {
                        map_value(func, &mut vmap, *clif, rir);
                    }
                }
                InstructionData::Jump { destination, .. } => {
                    let target = *block_map.get(&destination.block(&func.dfg.value_lists))?;
                    let args = block_args(func, &destination, &vmap)?;
                    builder.jump(rb, target, args);
                }
                InstructionData::Brif { arg, blocks, .. } => {
                    let c = as_bool(&mut builder, rb, lookup(func, &vmap, arg)?);
                    let then_block = *block_map.get(&blocks[0].block(&func.dfg.value_lists))?;
                    let then_args = block_args(func, &blocks[0], &vmap)?;
                    let else_block = *block_map.get(&blocks[1].block(&func.dfg.value_lists))?;
                    let else_args = block_args(func, &blocks[1], &vmap)?;
                    builder.brif(rb, c, then_block, then_args, else_block, else_args);
                }
                InstructionData::MultiAry { opcode: Opcode::Return, args } => {
                    let vals = args
                        .as_slice(&func.dfg.value_lists)
                        .iter()
                        .map(|v| lookup(func, &vmap, *v))
                        .collect::<Option<Vec<_>>>()?;
                    builder.ret(rb, vals);
                }
                InstructionData::NullAry { opcode: Opcode::Nop } => {}
                _ => return None,
            }
        }
    }

    let mut out = builder.into_func();
    out.externals = externals;
    rakit_ir::verify(&out).ok()?;
    Some(out)
}

/// Lower an RIR function back to CLIF, reusing `sig` for the calling convention
/// and ABI parameter details. Returns `None` on any inconsistency.
pub fn export(rir: &RirFunction, sig: &Signature) -> Option<ClifFunction> {
    let entry_params = &rir.block(rir.entry).params;
    if sig.params.len() != entry_params.len() || sig.returns.len() != rir.ret.len() {
        return None;
    }
    for (p, rv) in sig.params.iter().zip(entry_params) {
        if map_type_to_clif(rir.ty(*rv))? != p.value_type {
            return None;
        }
    }
    for (p, t) in sig.returns.iter().zip(&rir.ret) {
        if map_type_to_clif(*t)? != p.value_type {
            return None;
        }
    }

    let mut func = ClifFunction::new();
    func.signature = sig.clone();

    let mut call_refs = Vec::with_capacity(rir.externals.len());
    for ext in &rir.externals {
        let mut callee = Signature::new(CallConv::SystemV);
        for ty in &ext.params {
            callee.params.push(AbiParam::new(map_type_to_clif(*ty)?));
        }
        for ty in &ext.rets {
            callee.returns.push(AbiParam::new(map_type_to_clif(*ty)?));
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

    let mut slot_map: Vec<ClifStackSlot> = Vec::with_capacity(rir.stack_slots.len());
    for slot in &rir.stack_slots {
        let align_shift = slot.align.max(1).trailing_zeros() as u8;
        let data = ClifStackSlotData::new(StackSlotKind::ExplicitSlot, slot.size, align_shift);
        slot_map.push(func.create_sized_stack_slot(data));
    }

    let mut block_map: Vec<ClifBlock> = Vec::with_capacity(rir.blocks.len());
    let mut vmap: HashMap<RirValue, ClifValue> = HashMap::new();
    for rb in &rir.blocks {
        let cb = func.dfg.make_block();
        func.layout.append_block(cb);
        for p in &rb.params {
            let ty = map_type_to_clif(rir.ty(*p))?;
            let cv = func.dfg.append_block_param(cb, ty);
            vmap.insert(*p, cv);
        }
        block_map.push(cb);
    }

    for (bi, rb) in rir.blocks.iter().enumerate() {
        let cb = block_map[bi];
        for inst in &rb.insts {
            match inst {
                RirInst::Nop(r, ty) => {
                    let ct = map_type_to_clif(*ty)?;
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
                    let ct = map_type_to_clif(*ty)?;
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
                RirInst::Bin(r, op, ty, x, y) => {
                    let ct = map_type_to_clif(*ty)?;
                    let ci = func.dfg.make_inst(InstructionData::Binary {
                        opcode: map_binop_to_clif(*op)?,
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
                        cond: map_intcc_to_clif(*cc)?,
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
                        cond: map_floatcc_to_clif(*cc)?,
                    });
                    if func.dfg.make_inst_results(ci, types::I8) != 1 {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    vmap.insert(*r, func.dfg.inst_results(ci)[0]);
                }
                RirInst::Select(r, ty, c, x, y) => {
                    let ct = map_type_to_clif(*ty)?;
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
                RirInst::Load(r, ty, addr, offset) => {
                    let ct = map_type_to_clif(*ty)?;
                    let flags = func.dfg.mem_flags.insert(MemFlagsData::new()).ok()?;
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
                RirInst::Store(ty, addr, offset, value) => {
                    let _ = map_type_to_clif(*ty)?;
                    let flags = func.dfg.mem_flags.insert(MemFlagsData::new()).ok()?;
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
                        Some(ty) => map_type_to_clif(*ty)?,
                        None => types::I8,
                    };
                    let count = func.dfg.make_inst_results(ci, ctrl);
                    if count != ext.rets.len() || count != results.len() {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    let clif_results: Vec<_> = func.dfg.inst_results(ci).to_vec();
                    for (rir, clif) in results.iter().zip(clif_results) {
                        vmap.insert(*rir, clif);
                    }
                }
                RirInst::CallIndirect { results, params, rets, call_conv, callee, args } => {
                    let mut callee_sig = Signature::new(call_conv_from_rir(*call_conv)?);
                    for ty in params {
                        callee_sig.params.push(AbiParam::new(map_type_to_clif(*ty)?));
                    }
                    for ty in rets {
                        callee_sig.returns.push(AbiParam::new(map_type_to_clif(*ty)?));
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
                        Some(ty) => map_type_to_clif(*ty)?,
                        None => types::I8,
                    };
                    let count = func.dfg.make_inst_results(ci, ctrl);
                    if count != rets.len() || count != results.len() {
                        return None;
                    }
                    func.layout.append_inst(ci, cb);
                    let clif_results: Vec<_> = func.dfg.inst_results(ci).to_vec();
                    for (rir, clif) in results.iter().zip(clif_results) {
                        vmap.insert(*rir, clif);
                    }
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

fn map_value(
    func: &ClifFunction,
    vmap: &mut HashMap<ClifValue, RirValue>,
    c: ClifValue,
    r: RirValue,
) {
    vmap.insert(c, r);
    let canon = func.dfg.resolve_aliases(c);
    if canon != c {
        vmap.insert(canon, r);
    }
}

fn lookup(
    func: &ClifFunction,
    vmap: &HashMap<ClifValue, RirValue>,
    v: ClifValue,
) -> Option<RirValue> {
    vmap.get(&func.dfg.resolve_aliases(v)).copied().or_else(|| vmap.get(&v).copied())
}

fn single_result(func: &ClifFunction, inst: ClifInst) -> Option<ClifValue> {
    let results = func.dfg.inst_results(inst);
    if results.len() == 1 { Some(results[0]) } else { None }
}

fn block_args(
    func: &ClifFunction,
    bc: &BlockCall,
    vmap: &HashMap<ClifValue, RirValue>,
) -> Option<Vec<RirValue>> {
    bc.args(&func.dfg.value_lists)
        .map(|a| match a {
            BlockArg::Value(v) => lookup(func, vmap, v),
            _ => None,
        })
        .collect()
}

fn make_block_call(
    func: &mut ClifFunction,
    target: ClifBlock,
    args: &[RirValue],
    vmap: &HashMap<RirValue, ClifValue>,
) -> Option<BlockCall> {
    let barg: Vec<BlockArg> =
        args.iter().map(|v| vmap.get(v).copied().map(BlockArg::Value)).collect::<Option<_>>()?;
    Some(BlockCall::new(target, barg, &mut func.dfg.value_lists))
}

fn map_type_to_rir(t: ClifType) -> Option<RirType> {
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

fn map_type_to_clif(t: RirType) -> Option<ClifType> {
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

fn map_binop_to_clif(op: BinOp) -> Option<Opcode> {
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

fn map_intcc(cc: ClifIntCC) -> Option<RirIntCc> {
    Some(match cc {
        ClifIntCC::Equal => RirIntCc::Eq,
        ClifIntCC::NotEqual => RirIntCc::Ne,
        ClifIntCC::SignedLessThan => RirIntCc::Slt,
        ClifIntCC::SignedLessThanOrEqual => RirIntCc::Sle,
        ClifIntCC::SignedGreaterThan => RirIntCc::Sgt,
        ClifIntCC::SignedGreaterThanOrEqual => RirIntCc::Sge,
        ClifIntCC::UnsignedLessThan => RirIntCc::Ult,
        ClifIntCC::UnsignedLessThanOrEqual => RirIntCc::Ule,
        ClifIntCC::UnsignedGreaterThan => RirIntCc::Ugt,
        ClifIntCC::UnsignedGreaterThanOrEqual => RirIntCc::Uge,
    })
}

fn map_intcc_to_clif(cc: RirIntCc) -> Option<ClifIntCC> {
    Some(match cc {
        RirIntCc::Eq => ClifIntCC::Equal,
        RirIntCc::Ne => ClifIntCC::NotEqual,
        RirIntCc::Slt => ClifIntCC::SignedLessThan,
        RirIntCc::Sle => ClifIntCC::SignedLessThanOrEqual,
        RirIntCc::Sgt => ClifIntCC::SignedGreaterThan,
        RirIntCc::Sge => ClifIntCC::SignedGreaterThanOrEqual,
        RirIntCc::Ult => ClifIntCC::UnsignedLessThan,
        RirIntCc::Ule => ClifIntCC::UnsignedLessThanOrEqual,
        RirIntCc::Ugt => ClifIntCC::UnsignedGreaterThan,
        RirIntCc::Uge => ClifIntCC::UnsignedGreaterThanOrEqual,
    })
}

fn map_floatcc(cc: ClifFloatCC) -> Option<RirFloatCc> {
    Some(match cc {
        ClifFloatCC::Equal => RirFloatCc::Eq,
        ClifFloatCC::NotEqual => RirFloatCc::Ne,
        ClifFloatCC::LessThan => RirFloatCc::Lt,
        ClifFloatCC::LessThanOrEqual => RirFloatCc::Le,
        ClifFloatCC::GreaterThan => RirFloatCc::Gt,
        ClifFloatCC::GreaterThanOrEqual => RirFloatCc::Ge,
        _ => return None,
    })
}

fn map_floatcc_to_clif(cc: RirFloatCc) -> Option<ClifFloatCC> {
    Some(match cc {
        RirFloatCc::Eq => ClifFloatCC::Equal,
        RirFloatCc::Ne => ClifFloatCC::NotEqual,
        RirFloatCc::Lt => ClifFloatCC::LessThan,
        RirFloatCc::Le => ClifFloatCC::LessThanOrEqual,
        RirFloatCc::Gt => ClifFloatCC::GreaterThan,
        RirFloatCc::Ge => ClifFloatCC::GreaterThanOrEqual,
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

    fn new_func(params: &[Type], rets: &[Type]) -> (Function, Block) {
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

    fn iconst(func: &mut Function, b: Block, ty: Type, v: i64) -> Value {
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

    fn ret(func: &mut Function, b: Block, vals: &[Value]) {
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

    fn new_func(params: &[ClifType], rets: &[ClifType]) -> (ClifFunction, ClifBlock) {
        let mut func = ClifFunction::new();
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
}
