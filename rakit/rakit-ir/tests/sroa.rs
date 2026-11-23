use rakit_ir::ir::{BinOp, Inst, Type};
use rakit_ir::{Builder, Val, run, sroa, verify};

#[test]
fn promotes_straight_line_slot() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(sroa::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert!(f.stack_slots.is_empty());
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Load(..))));
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Store(..))));
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::StackAddr(..))));
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
}

#[test]
fn does_not_promote_cross_block_diamond() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::Bool]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let cond = b.func().block(e).params[1];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);

    let then_block = b.add_block(vec![]);
    let else_block = b.add_block(vec![]);
    let join = b.add_block(vec![]);
    b.brif(e, cond, then_block, vec![], else_block, vec![]);

    let ten = b.iconst(then_block, Type::I64, 10);
    b.store(then_block, Type::I64, addr, 0, ten);
    b.jump(then_block, join, vec![]);

    let twenty = b.iconst(else_block, Type::I64, 20);
    b.store(else_block, Type::I64, addr, 0, twenty);
    b.jump(else_block, join, vec![]);

    let y = b.load(join, Type::I64, addr, 0);
    b.ret(join, vec![y]);

    let mut f = b.into_func();
    assert!(!sroa::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(f.stack_slots.len(), 1);
    assert_eq!(run(&f, &[Val::I(0), Val::B(true)]).unwrap(), vec![Val::I(10)]);
    assert_eq!(run(&f, &[Val::I(0), Val::B(false)]).unwrap(), vec![Val::I(20)]);
    let _ = x;
}

#[test]
fn does_not_promote_uninitialized_slot() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    let y = b.load(e, Type::I64, addr, 0);
    b.store(e, Type::I64, addr, 0, x);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!sroa::run(&mut f));
    assert_eq!(f.stack_slots.len(), 1);
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(0)]);
}

#[test]
fn does_not_promote_dynamic_slot() {
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
    assert!(!sroa::run(&mut f));
    assert_eq!(f.stack_slots.len(), 1);
    assert_eq!(run(&f, &[Val::I(42), Val::I(1)]).unwrap(), vec![Val::I(42)]);
}

#[test]
fn removes_unused_slot() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let _slot = b.add_stack_slot(8, 8);
    b.ret(e, vec![x]);

    let mut f = b.into_func();
    assert!(sroa::run(&mut f));
    assert!(f.stack_slots.is_empty());
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(42)]);
}

#[test]
fn does_not_promote_cross_block_loop() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);

    let header = b.add_block(vec![Type::Bool]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![]);
    let t = b.iconst(e, Type::Bool, 1);
    b.jump(e, header, vec![t]);

    let flag = b.func().block(header).params[0];
    let y = b.load(header, Type::I64, addr, 0);
    let one = b.iconst(header, Type::I64, 1);
    let z = b.bin(header, BinOp::Add, Type::I64, y, one);
    b.store(header, Type::I64, addr, 0, z);
    b.brif(header, flag, body, vec![], exit, vec![]);

    let f0 = b.iconst(body, Type::Bool, 0);
    b.jump(body, header, vec![f0]);

    let out = b.load(exit, Type::I64, addr, 0);
    b.ret(exit, vec![out]);

    let mut f = b.into_func();
    assert!(!sroa::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(f.stack_slots.len(), 1);
    assert_eq!(run(&f, &[Val::I(0)]).unwrap(), vec![Val::I(2)]);
}

#[test]
fn does_not_promote_overlapping_accesses() {
    let mut b = Builder::new(vec![Type::I32], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I64, addr, 0, x);
    let seven = b.iconst(e, Type::I32, 7);
    b.store(e, Type::I32, addr, 0, seven);
    let y = b.load(e, Type::I32, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!sroa::run(&mut f));
    assert_eq!(f.stack_slots.len(), 1);
    assert_eq!(run(&f, &[Val::I(42)]).unwrap(), vec![Val::I(7)]);
}

#[test]
fn promotes_i128_slot() {
    let mut b = Builder::new(vec![Type::I128], vec![Type::I128]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(16, 16);
    let addr = b.stack_addr(e, slot, 0);
    b.store(e, Type::I128, addr, 0, x);
    let y = b.load(e, Type::I128, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(sroa::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert!(f.stack_slots.is_empty());
    let value = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210u128;
    assert_eq!(run(&f, &[Val::I128(value)]).unwrap(), vec![Val::I128(value)]);
}

#[test]
fn does_not_promote_dynamic_one_element_slot() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(8, 8);
    let base = b.stack_addr(e, slot, 0);
    let zero = b.iconst(e, Type::I64, 0);
    let eight = b.iconst(e, Type::I64, 8);
    let scaled = b.bin(e, BinOp::Mul, Type::I64, zero, eight);
    let addr = b.bin(e, BinOp::Add, Type::I64, base, scaled);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    assert!(!sroa::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(f.stack_slots.len(), 1);
}

#[test]
fn does_not_promote_dynamic_multi_element_slot() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let slot = b.add_stack_slot(16, 8);
    let base = b.stack_addr(e, slot, 0);
    let zero = b.iconst(e, Type::I64, 0);
    let eight = b.iconst(e, Type::I64, 8);
    let scaled = b.bin(e, BinOp::Mul, Type::I64, zero, eight);
    let addr = b.bin(e, BinOp::Add, Type::I64, base, scaled);
    b.store(e, Type::I64, addr, 0, x);
    let y = b.load(e, Type::I64, addr, 0);
    b.ret(e, vec![y]);

    let mut f = b.into_func();
    sroa::run(&mut f);
    assert_eq!(f.stack_slots.len(), 1);
}
