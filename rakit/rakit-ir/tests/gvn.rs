use rakit_ir::ir::{BinOp, Inst, Type};
use rakit_ir::{gvn, optimize, run, verify, Builder, Val};

#[test]
fn merges_duplicate_add() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let a = b.bin(e, BinOp::Add, Type::I64, x, y);
    let c = b.bin(e, BinOp::Add, Type::I64, x, y);
    let sum = b.bin(e, BinOp::Add, Type::I64, a, c);
    b.ret(e, vec![sum]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[Val::I(2), Val::I(3)]).unwrap(), vec![Val::I(10)]);
    assert!(gvn::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(2), Val::I(3)]).unwrap(), vec![Val::I(10)]);
    optimize(&mut f);
    assert_eq!(run(&f, &[Val::I(2), Val::I(3)]).unwrap(), vec![Val::I(10)]);
    assert!(f.blocks[0]
        .insts
        .iter()
        .any(|i| matches!(i, Inst::Copy(..))));
}

#[test]
fn does_not_merge_different_types() {
    let mut b = Builder::new(
        vec![Type::I64],
        vec![Type::I32, Type::I32, Type::I64, Type::I64],
    );
    let e = b.func().entry;
    let x32 = b.func().block(e).params[0];
    let y32 = b.func().block(e).params[1];
    let x64 = b.func().block(e).params[2];
    let y64 = b.func().block(e).params[3];
    let _a32 = b.bin(e, BinOp::Add, Type::I32, x32, y32);
    let a64 = b.bin(e, BinOp::Add, Type::I64, x64, y64);
    let sum = b.bin(e, BinOp::Add, Type::I64, a64, a64);
    b.ret(e, vec![sum]);

    let mut f = b.into_func();
    assert!(!gvn::run(&mut f));
}

#[test]
fn merges_duplicate_constants() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let a = b.iconst(e, Type::I64, 7);
    let c = b.iconst(e, Type::I64, 7);
    let sum = b.bin(e, BinOp::Add, Type::I64, a, c);
    b.ret(e, vec![sum]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(14)]);
    assert!(gvn::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(14)]);
}

#[test]
fn merges_duplicate_select() {
    let mut b = Builder::new(
        vec![Type::I64],
        vec![Type::Bool, Type::I64, Type::I64],
    );
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let x = b.func().block(e).params[1];
    let y = b.func().block(e).params[2];
    let s1 = b.select(e, Type::I64, c, x, y);
    let s2 = b.select(e, Type::I64, c, x, y);
    let sum = b.bin(e, BinOp::Add, Type::I64, s1, s2);
    b.ret(e, vec![sum]);

    let mut f = b.into_func();
    assert_eq!(
        run(&f, &[Val::B(true), Val::I(4), Val::I(9)]).unwrap(),
        vec![Val::I(8)]
    );
    assert!(gvn::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(
        run(&f, &[Val::B(true), Val::I(4), Val::I(9)]).unwrap(),
        vec![Val::I(8)]
    );
}

#[test]
fn does_not_merge_across_blocks() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let b1 = b.add_block(vec![]);
    let b2 = b.add_block(vec![]);
    b.jump(e, b1, vec![]);
    let _a = b.bin(b1, BinOp::Add, Type::I64, x, y);
    b.jump(b1, b2, vec![]);
    let c = b.bin(b2, BinOp::Add, Type::I64, x, y);
    b.ret(b2, vec![c]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[Val::I(2), Val::I(3)]).unwrap(), vec![Val::I(5)]);
    assert!(!gvn::run(&mut f));
}
