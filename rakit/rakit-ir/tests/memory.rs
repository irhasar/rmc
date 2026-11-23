use rakit_ir::ir::{BinOp, Inst, Type};
use rakit_ir::{optimize, run, verify, Builder, Val};

#[test]
fn store_load_roundtrip() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
}

#[test]
fn stack_slot_offsets() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let slot = b.add_stack_slot(16, 8);
    let lo = b.stack_addr(e, slot, 0);
    let hi = b.stack_addr(e, slot, 8);
    b.store(e, Type::I64, lo, 0, x);
    b.store(e, Type::I64, hi, 0, y);
    let a = b.load(e, Type::I64, lo, 0);
    let c = b.load(e, Type::I64, hi, 0);
    let sum = b.bin(e, BinOp::Add, Type::I64, a, c);
    b.ret(e, vec![sum]);

    let f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(
        run(&f, &[Val::I(3), Val::I(4)]).unwrap(),
        vec![Val::I(7)]
    );
}

#[test]
fn optimize_keeps_memory_ops() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let _unused = b.load(e, Type::I64, addr, 0);
    let zero = b.iconst(e, Type::I64, 0);
    b.ret(e, vec![zero]);

    let mut f = b.into_func();
    let _ = optimize(&mut f);
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Load(..))));
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
}
