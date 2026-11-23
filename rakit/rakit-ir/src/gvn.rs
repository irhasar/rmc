//! Local value numbering: replace a repeated pure instruction in a block with
//! a copy of the first result. Instructions are keyed by opcode, type and
//! operands, so different types or operands never merge.

use std::collections::HashMap;

use crate::ir::{BinOp, FloatCc, Function, Inst, IntCc, Type, Value};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Iconst(Type, i64),
    F32const(u32),
    F64const(u64),
    Copy(Type, Value),
    Bin(BinOp, Type, Value, Value),
    Icmp(IntCc, Type, Value, Value),
    Fcmp(FloatCc, Type, Value, Value),
    Select(Type, Value, Value, Value),
}

pub fn run(func: &mut Function) -> bool {
    let mut changed = false;
    for bi in 0..func.blocks.len() {
        let mut table: HashMap<Key, Value> = HashMap::new();
        let mut i = 0;
        while i < func.blocks[bi].insts.len() {
            let inst = func.blocks[bi].insts[i].clone();
            let Some(key) = key_of(&inst) else {
                i += 1;
                continue;
            };
            if let Some(&existing) = table.get(&key) {
                let r = inst.result_value().expect("keyed instruction has a result");
                let ty = func.values[r.0 as usize];
                func.blocks[bi].insts[i] = Inst::Copy(r, ty, existing);
                changed = true;
            } else if let Some(r) = inst.result_value() {
                table.insert(key, r);
            }
            i += 1;
        }
    }
    changed
}

fn key_of(inst: &Inst) -> Option<Key> {
    Some(match inst {
        Inst::Iconst(_, ty, v) => Key::Iconst(*ty, *v),
        Inst::F32const(_, v) => Key::F32const(v.to_bits()),
        Inst::F64const(_, v) => Key::F64const(v.to_bits()),
        Inst::Copy(_, ty, x) => Key::Copy(*ty, *x),
        Inst::Bin(_, op, ty, x, y) => Key::Bin(*op, *ty, *x, *y),
        Inst::Icmp(_, cc, ty, x, y) => Key::Icmp(*cc, *ty, *x, *y),
        Inst::Fcmp(_, cc, ty, x, y) => Key::Fcmp(*cc, *ty, *x, *y),
        Inst::Select(_, ty, c, x, y) => Key::Select(*ty, *c, *x, *y),
        _ => return None,
    })
}
