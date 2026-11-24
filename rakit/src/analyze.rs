//! SSA analysis

use rustc_index::IndexVec;
use rustc_middle::mir::StatementKind::*;

use crate::prelude::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SsaKind {
    NotSsa,
    MaybeSsa,
}

impl SsaKind {
    pub(crate) fn is_ssa<'tcx>(self, fx: &FunctionCx<'_, '_, 'tcx>, ty: Ty<'tcx>) -> bool {
        self == SsaKind::MaybeSsa && (fx.rakit_type(ty).is_some() || fx.rakit_pair_type(ty).is_some())
    }
}

pub(crate) fn analyze(fx: &FunctionCx<'_, '_, '_>) -> IndexVec<Local, SsaKind> {
    let mut flag_map =
        fx.mir.local_decls.iter().map(|_| SsaKind::MaybeSsa).collect::<IndexVec<Local, SsaKind>>();

    for bb in fx.mir.basic_blocks.iter() {
        for stmt in bb.statements.iter() {
            let Assign(place_and_rval) = &stmt.kind else { continue };
            match &place_and_rval.1 {
                // A shared borrow of a scalar local does not prevent keeping it in a variable:
                // we materialize a stack slot lazily at the borrow site (see `CPlace::to_ptr`).
                // The borrow checker guarantees the local cannot be mutated while the borrow is
                // live, so spilling the value there is sound and reads through the reference see
                // the right value. This keeps hot locals in registers even when they are later
                // borrowed, e.g. `println!("{}", acc)` only borrows `acc` after the loop.
                Rvalue::Ref(_, BorrowKind::Shared, place) => {
                    let ty = fx.monomorphize(fx.mir.local_decls[place.local].ty);
                    if !SsaKind::MaybeSsa.is_ssa(fx, ty) {
                        flag_map[place.local] = SsaKind::NotSsa;
                    }
                }
                // A mutable or raw borrow that points *into* the local's own storage (the local
                // itself, or a field/array element of it) hands out a pointer through which the
                // local can be written, so the variable would go stale; keep it in a stack slot.
                //
                // A borrow that dereferences the local first (e.g. `&mut *p` for a pointer `p`)
                // only *reads* the local's value to form the address and writes through the
                // pointer it holds, not into the local's storage. Such a local can stay in a
                // variable. This is what keeps a `Vec`'s header (ptr/len/cap) in registers across
                // an indexing loop instead of reloading it from a stack slot every iteration.
                Rvalue::Ref(_, _, place) | Rvalue::RawPtr(_, place) => {
                    if !place.projection.iter().any(|elem| matches!(elem, PlaceElem::Deref)) {
                        flag_map[place.local] = SsaKind::NotSsa;
                    }
                }
                _ => {}
            }
        }
    }

    flag_map
}
