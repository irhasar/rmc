//! Inline `%Memcpy` calls whose size is a small constant.
//!
//! `ptr::copy_nonoverlapping::<u8>(src, dst, size_of::<__m256i>())` reaches the
//! IR as a call to a `#[inline]` generic helper whose body passes a *runtime*
//! count to the `memcpy` libcall. The inliner copies that body into the caller,
//! but the libcall itself stays: a `%Memcpy` of 16 or 32 bytes per AVX2 kernel
//! iteration costs a call, a size check and a general-purpose byte loop, while
//! two 16-byte vector moves do the same work inline.
//!
//! The pass rewrites such a call into load/store pairs, widest first. It runs
//! before store forwarding so the destination slot stays visible to it.

use rakit_codegen::cursor::{Cursor, FuncCursor};
use rakit_codegen::ir::{self, InstBuilder, LibCall, MemFlagsData, Opcode, types};

use crate::pass::{Pass, PassCtx};

/// Largest copy that is worth inlining. Above this the libcall's byte loop wins
/// over unrolled vector moves.
const MAX_BYTES: u64 = 128;

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
                let Some(plan) = chunk_plan(bytes) else { continue };
                let mut pos = FuncCursor::new(&mut pctx.ctx.func).at_inst(inst);
                for (offset, ty) in plan {
                    let val = pos.ins().load(ty, MemFlagsData::new(), src, offset);
                    pos.ins().store(MemFlagsData::new(), val, dst, offset);
                }
                pos.func.layout.remove_inst(inst);
                changed = true;
            }
        }
        changed
    }
}

/// Split a constant copy size into `(offset, type)` moves, widest first.
///
/// rakit's aggregate copies are 40, 56 or 72 bytes as often as they are a clean
/// multiple of 16 (`Option<Vec>`, `RawTableInner` bookkeeping), and the tail is
/// what forced the libcall there. `None` for a size not worth inlining.
fn chunk_plan(bytes: u64) -> Option<Vec<(i32, types::Type)>> {
    if bytes == 0 || bytes > MAX_BYTES {
        return None;
    }
    let mut plan = Vec::new();
    let mut offset = 0u64;
    let mut left = bytes;
    while left >= 16 {
        plan.push((offset as i32, types::I8X16));
        offset += 16;
        left -= 16;
    }
    for (size, ty) in [(8, types::I64), (4, types::I32), (2, types::I16), (1, types::I8)] {
        if left >= size {
            plan.push((offset as i32, ty));
            offset += size;
            left -= size;
        }
    }
    (left == 0).then_some(plan)
}
