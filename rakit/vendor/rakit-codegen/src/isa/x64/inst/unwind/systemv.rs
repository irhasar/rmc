//! Unwind information for System V ABI (x86-64).

use crate::isa::unwind::systemv::RegisterMappingError;
use crate::machinst::{Reg, RegClass};
use gimli::{Encoding, Format, Register, X86_64, write::CommonInformationEntry};

/// Creates a new x86-64 common information entry (CIE).
pub fn create_cie() -> CommonInformationEntry {
    use gimli::write::CallFrameInstruction;

    let mut entry = CommonInformationEntry::new(
        Encoding {
            address_size: 8,
            format: Format::Dwarf32,
            version: 1,
        },
        1,  // Code alignment factor
        -8, // Data alignment factor
        X86_64::RA,
    );

    // Every frame will start with the call frame address (CFA) at RSP+8
    // It is +8 to account for the push of the return address by the call instruction
    entry.add_instruction(CallFrameInstruction::Cfa(X86_64::RSP, 8));

    // Every frame will start with the return address at RSP (CFA-8 = RSP+8-8 = RSP)
    entry.add_instruction(CallFrameInstruction::Offset(X86_64::RA, -8));

    entry
}

/// Map Rakit registers to their corresponding Gimli registers.
pub fn map_reg(reg: Reg) -> Result<Register, RegisterMappingError> {
    // Mapping from https://github.com/bytecodealliance/rakit/pull/902 by @iximeow
    const X86_GP_REG_MAP: [gimli::Register; 16] = [
        X86_64::RAX,
        X86_64::RCX,
        X86_64::RDX,
        X86_64::RBX,
        X86_64::RSP,
        X86_64::RBP,
        X86_64::RSI,
        X86_64::RDI,
        X86_64::R8,
        X86_64::R9,
        X86_64::R10,
        X86_64::R11,
        X86_64::R12,
        X86_64::R13,
        X86_64::R14,
        X86_64::R15,
    ];
    const X86_XMM_REG_MAP: [gimli::Register; 16] = [
        X86_64::XMM0,
        X86_64::XMM1,
        X86_64::XMM2,
        X86_64::XMM3,
        X86_64::XMM4,
        X86_64::XMM5,
        X86_64::XMM6,
        X86_64::XMM7,
        X86_64::XMM8,
        X86_64::XMM9,
        X86_64::XMM10,
        X86_64::XMM11,
        X86_64::XMM12,
        X86_64::XMM13,
        X86_64::XMM14,
        X86_64::XMM15,
    ];

    match reg.class() {
        RegClass::Int => {
            // x86 GP registers have a weird mapping to DWARF registers, so we use a
            // lookup table.
            Ok(X86_GP_REG_MAP[reg.to_real_reg().unwrap().hw_enc() as usize])
        }
        RegClass::Float => Ok(X86_XMM_REG_MAP[reg.to_real_reg().unwrap().hw_enc() as usize]),
        RegClass::Vector => unreachable!(),
    }
}

pub(crate) struct RegisterMapper;

impl crate::isa::unwind::systemv::RegisterMapper<Reg> for RegisterMapper {
    fn map(&self, reg: Reg) -> Result<u16, RegisterMappingError> {
        Ok(map_reg(reg)?.0)
    }
    fn fp(&self) -> Option<u16> {
        Some(X86_64::RBP.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::Context;
    use crate::cursor::{Cursor, FuncCursor};
    use crate::ir::{
        AbiParam, Function, InstBuilder, Signature, StackSlotData, StackSlotKind, types,
    };
    use crate::isa::{CallConv, lookup};
    use crate::settings::{Configurable, Flags, builder};
    use gimli::write::Address;
    use target_lexicon::triple;

    #[test]
    fn test_simple_func() {
        let mut b = builder();
        b.set("preserve_frame_pointers", "true").unwrap();
        let isa = lookup(triple!("x86_64"))
            .expect("expect x86 ISA")
            .finish(Flags::new(b))
            .expect("expect backend creation to succeed");

        let mut context = Context::for_function(create_function(
            CallConv::SystemV,
            Some(StackSlotData::new(StackSlotKind::ExplicitSlot, 64, 0)),
        ));

        let code = context
            .compile(&*isa, &mut Default::default())
            .expect("expected compilation");

        let fde = match code
            .create_unwind_info(isa.as_ref())
            .expect("can create unwind info")
        {
            Some(crate::isa::unwind::UnwindInfo::SystemV(info)) => {
                info.to_fde(Address::Constant(1234))
            }
            _ => panic!("expected unwind information"),
        };

        assert_eq!(
            format!("{fde:?}"),
            "FrameDescriptionEntry { address: Constant(1234), length: 17, lsda: None, instructions: [(1, CfaOffset(16)), (1, Offset(Register(6), -16)), (4, CfaRegister(Register(6)))] }"
        );
    }

    /// The per-register rules of a frameless frame, derived from the prologue
    /// generator rather than from hand-written pairs: `gen_clobber_save` decides
    /// which register goes in which clobber slot, and a wrong order there would
    /// make the unwinder read the wrong slot.
    #[test]
    fn test_frameless_save_reg_offsets() {
        use crate::isa::unwind::UnwindInst;
        use crate::isa::unwind::systemv::{
            RegisterMapper, RegisterMappingError, create_unwind_info_from_insts,
        };
        use crate::isa;
        use crate::isa::x64::inst::regs;
        use crate::machinst::{ABIMachineSpec, FrameLayout, FunctionCalls, Writable};
        use crate::settings::builder;
        use crate::isa::x64::abi::X64ABIMachineSpec;

        struct Mapper;
        impl RegisterMapper<crate::machinst::Reg> for Mapper {
            fn map(&self, reg: crate::machinst::Reg) -> Result<u16, RegisterMappingError> {
                let real = reg.to_real_reg().expect("expected a real register");
                Ok(real.hw_enc() as u16)
            }
            // x86-64 has a frame register, so a frame that never pushes it must
            // not be described in terms of it.
            fn fp(&self) -> Option<u16> {
                Some(regs::rbp().to_real_reg().expect("real register").hw_enc() as u16)
            }
        }

        // Shape of a real frameless function: `sub $0x68` = 72 bytes of fixed
        // frame and stack slots plus a 32 byte clobber area, with rbx/r12/r13
        // saved at 0x48/0x50/0x58(%rsp).
        let layout = FrameLayout {
            incoming_args_size: 0,
            tail_args_size: 0,
            setup_area_size: 8,
            clobber_size: 32,
            fixed_frame_storage_size: 72,
            stackslots_size: 64,
            outgoing_args_size: 0,
            clobbered_callee_saves: vec![
                Writable::from_reg(regs::rbx().to_real_reg().expect("real register")),
                Writable::from_reg(regs::r12().to_real_reg().expect("real register")),
                Writable::from_reg(regs::r13().to_real_reg().expect("real register")),
            ],
            function_calls: FunctionCalls::Regular,
            ..Default::default()
        };
        let flags = Flags::new(builder());
        let mut insts = vec![
            (
                0,
                UnwindInst::DefineNewFrame {
                    offset_upward_to_caller_sp: 8,
                    offset_downward_to_clobbers: 32,
                },
            ),
            (4, UnwindInst::StackAlloc { size: 104 }),
        ];
        // `clobber_offset` is the offset *within* the clobber area; the store
        // itself goes to `fixed + outgoing + clobber_offset`.
        let base = layout.fixed_frame_storage_size + layout.outgoing_args_size;
        let mut next = 0;
        let mut saved = vec![];
        for inst in X64ABIMachineSpec::gen_clobber_save(isa::CallConv::SystemV, &flags, &layout) {
            let crate::isa::x64::inst::Inst::Unwind { inst } = inst else {
                continue;
            };
            let UnwindInst::SaveReg { clobber_offset, reg } = inst else {
                continue;
            };
            // Every clobber is 8 bytes here, so the slots must run 0, 8, 16 in
            // the order the layout lists the registers: a register paired with
            // another register's slot is exactly what this catches.
            assert_eq!(clobber_offset, next, "slot out of order for {reg:?}");
            assert!(base + clobber_offset < layout.setup_area_size + 104);
            assert_eq!(reg, layout.clobbered_callee_saves[saved.len()].to_reg());
            next += 8;
            saved.push(reg);
            insts.push((8 + 4 * saved.len() as u32, UnwindInst::SaveReg { clobber_offset, reg }));
        }
        assert_eq!(saved.len(), 3, "expected one SaveReg per clobbered register");

        let info = create_unwind_info_from_insts(&insts, 20, &Mapper).expect("unwind info");
        assert_eq!(
            format!("{:?}", info.to_fde(Address::Constant(0))),
            "FrameDescriptionEntry { address: Constant(0), length: 20, lsda: None, instructions: [(4, CfaOffset(112)), (12, Offset(Register(3), -40)), (16, Offset(Register(12), -32)), (20, Offset(Register(13), -24))] }"
        );
    }

    /// A function compiled without a frame pointer must describe its CFA in
    /// terms of RSP: RBP still holds the *caller's* frame pointer, so an
    /// FP-relative rule would point the unwinder at the caller's frame and lose
    /// every frame above it.
    #[test]
    fn test_frameless_func_uses_sp_relative_cfa() {
        let mut b = builder();
        b.set("preserve_frame_pointers", "false").unwrap();
        let isa = lookup(triple!("x86_64"))
            .expect("expect x86 ISA")
            .finish(Flags::new(b))
            .expect("expect backend creation to succeed");

        let mut context = Context::for_function(create_function(
            CallConv::SystemV,
            Some(StackSlotData::new(StackSlotKind::ExplicitSlot, 64, 0)),
        ));

        let code = context
            .compile(&*isa, &mut Default::default())
            .expect("expected compilation");

        let fde = match code
            .create_unwind_info(isa.as_ref())
            .expect("can create unwind info")
        {
            Some(crate::isa::unwind::UnwindInfo::SystemV(info)) => {
                info.to_fde(Address::Constant(1234))
            }
            _ => panic!("expected unwind information"),
        };
        let fde = format!("{fde:?}");

        assert!(!fde.contains("CfaRegister"), "CFA must stay SP-relative: {fde}");
        // CFA is RSP+8 at entry, then RSP+80 once the frame is allocated: 64
        // bytes of stack slot plus the 8 bytes of padding that keep the
        // outgoing argument area 16 byte aligned. The return address is always
        // at CFA-8, i.e. RSP+72.
        assert!(fde.contains("CfaOffset(80)"), "stack allocation not recorded: {fde}");
    }

    fn create_function(call_conv: CallConv, stack_slot: Option<StackSlotData>) -> Function {
        let mut func = Function::with_name_signature(Default::default(), Signature::new(call_conv));

        let block0 = func.dfg.make_block();
        let mut pos = FuncCursor::new(&mut func);
        pos.insert_block(block0);
        pos.ins().return_(&[]);

        if let Some(stack_slot) = stack_slot {
            func.sized_stack_slots.push(stack_slot);
        }

        func
    }

    #[test]
    fn test_multi_return_func() {
        let mut b = builder();
        b.set("preserve_frame_pointers", "true").unwrap();
        let isa = lookup(triple!("x86_64"))
            .expect("expect x86 ISA")
            .finish(Flags::new(b))
            .expect("expect backend creation to succeed");

        let mut context = Context::for_function(create_multi_return_function(CallConv::SystemV));

        let code = context
            .compile(&*isa, &mut Default::default())
            .expect("expected compilation");

        let fde = match code
            .create_unwind_info(isa.as_ref())
            .expect("can create unwind info")
        {
            Some(crate::isa::unwind::UnwindInfo::SystemV(info)) => {
                info.to_fde(Address::Constant(4321))
            }
            _ => panic!("expected unwind information"),
        };

        assert_eq!(
            format!("{fde:?}"),
            "FrameDescriptionEntry { address: Constant(4321), length: 22, lsda: None, instructions: [(1, CfaOffset(16)), (1, Offset(Register(6), -16)), (4, CfaRegister(Register(6)))] }"
        );
    }

    fn create_multi_return_function(call_conv: CallConv) -> Function {
        let mut sig = Signature::new(call_conv);
        sig.params.push(AbiParam::new(types::I32));
        let mut func = Function::with_name_signature(Default::default(), sig);

        let block0 = func.dfg.make_block();
        let v0 = func.dfg.append_block_param(block0, types::I32);
        let block1 = func.dfg.make_block();
        let block2 = func.dfg.make_block();

        let mut pos = FuncCursor::new(&mut func);
        pos.insert_block(block0);
        pos.ins().brif(v0, block2, &[], block1, &[]);

        pos.insert_block(block1);
        pos.ins().return_(&[]);

        pos.insert_block(block2);
        pos.ins().return_(&[]);

        func
    }
}
