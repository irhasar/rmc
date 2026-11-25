//! Inline `%Memcpy` calls whose size is a small constant.
//!
//! `ptr::copy_nonoverlapping::<u8>(src, dst, size_of::<__m256i>())` reaches the
//! IR as a call to a `#[inline]` generic helper whose body passes a *runtime*
//! count to the `memcpy` libcall. The inliner copies that body into the caller,
//! but the libcall itself stays: a `%Memcpy` of 16 or 32 bytes per AVX2 kernel
//! iteration costs a call, a size check and a general-purpose byte loop, while
//! two 16-byte vector moves do the same work inline.
//!
//! The pass rewrites such a call into `size / 16` load/store pairs. It runs
//! before store forwarding so the destination slot stays visible to it.

use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::{self, InstBuilder, LibCall, MemFlagsData, Opcode, types};

use crate::pass::{Pass, PassCtx};

/// Largest copy that is worth inlining. Above this the libcall's byte loop wins
/// over unrolled vector moves.
const MAX_BYTES: u64 = 64;

pub struct MemcpyInlinePass;

impl Pass for MemcpyInlinePass {
    fn name(&self) -> &'static str {
        "memcpy_inline"
    }

    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool {
        let mut changed = false;
        let blocks: Vec<ir::Block> = pctx.ctx.func.layout.blocks().collect();
        for block in blocks {
            let insts: Vec<ir::Inst> = pctx.ctx.func.layout.block_insts(block).collect();
            for inst in insts {
                if pctx.ctx.func.dfg.insts[inst].opcode() != Opcode::Call {
                    continue;
                }
                let ir::InstructionData::Call { func_ref, args, .. } = pctx.ctx.func.dfg.insts[inst]
                else {
                    continue;
                };
                if pctx.ctx.func.dfg.ext_funcs[func_ref].name
                    != ir::ExternalName::LibCall(LibCall::Memcpy)
                {
                    continue;
                }
                let args: Vec<ir::Value> = args.as_slice(&pctx.ctx.func.dfg.value_lists).to_vec();
                let [dst, src, size] = args[..] else { continue };
                let Some(bytes) = crate::passes::unroll::const_of(&pctx.ctx.func, size) else {
                    continue;
                };
                if bytes == 0 || bytes > MAX_BYTES || bytes % 16 != 0 {
                    continue;
                }
                let mut pos = FuncCursor::new(&mut pctx.ctx.func).at_inst(inst);
                let mut offset = 0i32;
                while u64::from(offset as u32) < bytes {
                    let val = pos.ins().load(types::I8X16, MemFlagsData::new(), src, offset);
                    pos.ins().store(MemFlagsData::new(), val, dst, offset);
                    offset += 16;
                }
                pos.func.layout.remove_inst(inst);
                changed = true;
            }
        }
        changed
    }
}
