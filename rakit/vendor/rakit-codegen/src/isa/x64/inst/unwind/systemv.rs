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
