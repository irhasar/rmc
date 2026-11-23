use rakit_ir::ir::{BinOp, Inst, Type};
use rakit_ir::{mem_forward, run, verify, Builder, Val};

#[test]
fn forwards_store_to_load() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(mem_forward::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Load(..))));
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
}

#[test]
fn forwards_across_blocks() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let body = b.add_block(vec![]);
    b.jump(e, body, vec![]);
    let y = b.load(body, Type::I64, addr, 0);
    b.ret(body, vec![y]);

    let mut f = b.into_func();
    assert!(mem_forward::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
    assert!(!f.blocks[1].insts.iter().any(|i| matches!(i, Inst::Load(..))));
}

#[test]
fn does_not_forward_escaping_slot() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    let _escaped = b.bin(e, BinOp::Sub, Type::I64, addr, addr);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!mem_forward::run(&mut f));
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Load(..))));
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
}

#[test]
fn removes_store_without_load() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let zero = b.iconst(e, Type::I64, 0);
    b.ret(e, vec![zero]);

    let mut f = b.into_func();
    assert!(mem_forward::run(&mut f));
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(0)]);
}

#[test]
fn keeps_store_with_overlapping_load() {
    let mut b = Builder::new(vec![Type::I32], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I32, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!mem_forward::run(&mut f));
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
}

#[test]
fn forwards_same_dynamic_address() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let i = b.func().block(e).params[1];
    let slot = b.add_stack_slot(64, 8);
    let base = b.stack_addr(e, slot, 0);
    let eight = b.iconst(e, Type::I64, 8);
    let scaled = b.bin(e, BinOp::Mul, Type::I64, i, eight);
    let addr = b.bin(e, BinOp::Add, Type::I64, base, scaled);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(mem_forward::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(42), Val::I(1)]).unwrap(), vec![Val::I(42)]);
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Load(..))));
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
}

#[test]
fn does_not_forward_different_dynamic_addresses() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let i = b.func().block(e).params[1];
    let j = b.func().block(e).params[2];
    let slot = b.add_stack_slot(64, 8);
    let base = b.stack_addr(e, slot, 0);
    let eight = b.iconst(e, Type::I64, 8);
    let si = b.bin(e, BinOp::Mul, Type::I64, i, eight);
    let sj = b.bin(e, BinOp::Mul, Type::I64, j, eight);
    let ai = b.bin(e, BinOp::Add, Type::I64, base, si);
    let aj = b.bin(e, BinOp::Add, Type::I64, base, sj);
    b.store(e, Type::I64, ai, 0, x);
    let y = b.load(e, Type::I64, aj, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!mem_forward::run(&mut f));
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
    assert_eq!(
        run(&f, &[Val::I(42), Val::I(1), Val::I(1)]).unwrap(),
        vec![Val::I(42)]
    );
}

#[test]
fn constant_store_not_forwarded_to_dynamic_load() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let i = b.func().block(e).params[1];
    let slot = b.add_stack_slot(64, 8);
    let base = b.stack_addr(e, slot, 0);
    let eight = b.iconst(e, Type::I64, 8);
    let scaled = b.bin(e, BinOp::Mul, Type::I64, i, eight);
    let addr = b.bin(e, BinOp::Add, Type::I64, base, scaled);
    b.store(e, Type::I64, base, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!mem_forward::run(&mut f));
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
    assert_eq!(run(&f, &[Val::I(42), Val::I(0)]).unwrap(), vec![Val::I(42)]);
}
