use rakit_ir::*;

#[test]
fn fold_and_interpret_integer() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let two = b.iconst(e, Type::I64, 2);
    let three = b.iconst(e, Type::I64, 3);
    let sum = b.bin(e, BinOp::Add, Type::I64, two, three);
    let four = b.iconst(e, Type::I64, 4);
    let prod = b.bin(e, BinOp::Mul, Type::I64, sum, four);
    b.ret(e, vec![prod]);

    let mut f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(20)]);

    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(20)]);

    let ret = f.blocks[0].terminator().unwrap().clone();
    let args = match ret {
        Inst::Return(a) => a,
        other => panic!("expected return, got {other:?}"),
    };
    let def = f.blocks[0]
        .insts
        .iter()
        .find(|i| i.result_value() == Some(args[0]))
        .unwrap();
    assert!(matches!(def, Inst::Iconst(_, _, 20)), "got {def:?}");
}

#[test]
fn branch_folding_picks_the_taken_edge() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let cond = b.iconst(e, Type::Bool, 1);
    b.brif(e, cond, t, vec![], f, vec![]);
    let one = b.iconst(t, Type::I64, 1);
    b.ret(t, vec![one]);
    let two = b.iconst(f, Type::I64, 2);
    b.ret(f, vec![two]);

    let mut func = b.into_func();
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(run(&func, &[]).unwrap(), vec![Val::I(1)]);

    assert!(optimize(&mut func));
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(run(&func, &[]).unwrap(), vec![Val::I(1)]);
    assert!(matches!(func.blocks[0].terminator().unwrap(), Inst::Jump(t2, _) if *t2 == t));
}

#[test]
fn verifier_rejects_use_before_def() {
    let mut f = Function::new(vec![Type::I64], vec![]);
    f.values.push(Type::I64); // v0
    f.values.push(Type::I64); // v1
    let e = f.entry;
    f.block_mut(e)
        .insts
        .push(Inst::Bin(Value(0), BinOp::Add, Type::I64, Value(1), Value(1)));
    f.block_mut(e).insts.push(Inst::Iconst(Value(1), Type::I64, 1));
    f.block_mut(e).insts.push(Inst::Return(vec![Value(0)]));

    let err = verify(&f).unwrap_err();
    assert!(err.iter().any(|e| e.contains("before")), "{err:?}");
}

#[test]
fn dce_removes_dead_code() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let a = b.iconst(e, Type::I64, 1);
    let c = b.iconst(e, Type::I64, 2);
    let _dead = b.bin(e, BinOp::Add, Type::I64, a, c);
    let k = b.iconst(e, Type::I64, 7);
    b.ret(e, vec![k]);

    let mut f = b.into_func();
    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Nop(..))));
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(7)]);
}

#[test]
fn float_folding() {
    let mut b = Builder::new(vec![Type::F64], vec![]);
    let e = b.func().entry;
    let x = b.f64const(e, 1.5);
    let y = b.f64const(e, 2.5);
    let sum = b.bin(e, BinOp::Fadd, Type::F64, x, y);
    let two = b.f64const(e, 2.0);
    let prod = b.bin(e, BinOp::Fmul, Type::F64, sum, two);
    b.ret(e, vec![prod]);

    let mut f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::F64(8.0)]);
}

#[test]
fn block_params_and_branch_args() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let t = b.add_block(vec![Type::I64]);
    let one = b.iconst(e, Type::I64, 1);
    b.jump(e, t, vec![one]);
    let p = b.func().block(t).params[0];
    b.ret(t, vec![p]);

    let f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(1)]);
}

#[test]
fn icmp_folding_and_select() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let a = b.iconst(e, Type::I64, 5);
    let c = b.iconst(e, Type::I64, 3);
    let cond = b.icmp(e, IntCc::Sgt, Type::I64, a, c);
    let x = b.iconst(e, Type::I64, 10);
    let y = b.iconst(e, Type::I64, 20);
    let sel = b.select(e, Type::I64, cond, x, y);
    b.ret(e, vec![sel]);

    let mut f = b.into_func();
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(10)]);
    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(10)]);
}

#[test]
fn icmp_same_operand_folds_to_true() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let cond = b.icmp(e, IntCc::Eq, Type::I64, x, x);
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    b.brif(e, cond, t, vec![], f, vec![]);
    let one = b.iconst(t, Type::I64, 1);
    b.ret(t, vec![one]);
    let two = b.iconst(f, Type::I64, 2);
    b.ret(f, vec![two]);

    let mut func = b.into_func();
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(run(&func, &[Val::I(42)]).unwrap(), vec![Val::I(1)]);
    assert!(optimize(&mut func));
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(run(&func, &[Val::I(42)]).unwrap(), vec![Val::I(1)]);
    assert!(!func.blocks[0].insts.iter().any(|i| matches!(i, Inst::Icmp(..))));
    assert!(matches!(func.blocks[0].terminator().unwrap(), Inst::Jump(t2, _) if *t2 == t));
}

#[test]
fn select_same_arms_folds() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let cond = b.icmp(e, IntCc::Eq, Type::I64, x, y);
    let sel = b.select(e, Type::I64, cond, x, x);
    b.ret(e, vec![sel]);

    let mut f = b.into_func();
    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(7), Val::I(9)]).unwrap(), vec![Val::I(7)]);
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Select(..))));
}

#[test]
fn bin_same_operand_folds() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let sub = b.bin(e, BinOp::Sub, Type::I64, x, x);
    let and = b.bin(e, BinOp::And, Type::I64, x, x);
    let sum = b.bin(e, BinOp::Add, Type::I64, sub, and);
    b.ret(e, vec![sum]);

    let mut f = b.into_func();
    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(123)]).unwrap(), vec![Val::I(123)]);
    assert!(!f.blocks[0].insts.iter().any(|i| matches!(
        i,
        Inst::Bin(_, BinOp::Sub, ..) | Inst::Bin(_, BinOp::And, ..)
    )));
}

#[test]
fn brif_to_same_target_folds_to_jump() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let cond = b.icmp(e, IntCc::Eq, Type::I64, x, y);
    let t = b.add_block(vec![]);
    b.brif(e, cond, t, vec![], t, vec![]);
    let five = b.iconst(t, Type::I64, 5);
    b.ret(t, vec![five]);

    let mut f = b.into_func();
    assert!(optimize(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(1), Val::I(2)]).unwrap(), vec![Val::I(5)]);
    assert!(matches!(f.blocks[0].terminator().unwrap(), Inst::Jump(t2, _) if *t2 == t));
}

#[test]
fn narrow_integers_wrap_and_sign_extend() {
    let mut b = Builder::new(vec![Type::I8, Type::Bool], vec![Type::I8]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let one = b.iconst(e, Type::I8, 1);
    let sum = b.bin(e, BinOp::Add, Type::I8, x, one);
    let neg_one = b.iconst(e, Type::I8, 255);
    let neg = b.icmp(e, IntCc::Slt, Type::I8, neg_one, one);
    b.ret(e, vec![sum, neg]);

    let f = b.into_func();
    assert_eq!(
        run(&f, &[Val::I(127)]).unwrap(),
        vec![Val::I(128), Val::B(true)]
    );
}

#[test]
fn narrow_shift_amount_wraps_to_width() {
    let mut b = Builder::new(vec![Type::I8], vec![Type::I8]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let eight = b.iconst(e, Type::I8, 8);
    let shifted = b.bin(e, BinOp::Shl, Type::I8, x, eight);
    b.ret(e, vec![shifted]);

    let f = b.into_func();
    assert_eq!(run(&f, &[Val::I(1)]).unwrap(), vec![Val::I(1)]);
}

#[test]
fn i32_add_wraps() {
    let mut b = Builder::new(vec![Type::I32], vec![Type::I32]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let one = b.iconst(e, Type::I32, 1);
    let sum = b.bin(e, BinOp::Add, Type::I32, x, one);
    b.ret(e, vec![sum]);

    let f = b.into_func();
    assert_eq!(run(&f, &[Val::I(0xFFFF_FFFF)]).unwrap(), vec![Val::I(0)]);
}

#[test]
fn narrow_constant_folding_wraps() {
    let mut b = Builder::new(vec![Type::I8, Type::Bool, Type::Bool], vec![]);
    let e = b.func().entry;
    let a = b.iconst(e, Type::I8, 255);
    let one = b.iconst(e, Type::I8, 1);
    let sum = b.bin(e, BinOp::Add, Type::I8, a, one);
    let slt = b.icmp(e, IntCc::Slt, Type::I8, a, one);
    let ult = b.icmp(e, IntCc::Ult, Type::I8, a, one);
    b.ret(e, vec![sum, slt, ult]);

    let mut f = b.into_func();
    assert!(optimize(&mut f));
    assert_eq!(
        run(&f, &[]).unwrap(),
        vec![Val::I(0), Val::B(true), Val::B(false)]
    );
}
