use rakit_ir::ir::{BinOp, Function, Inst, Type};
use rakit_ir::{if_convert, optimize, run, verify, Builder, Val};

fn build_constant_diamond() -> Function {
    let mut b = Builder::new(vec![Type::I64], vec![Type::Bool]);
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![Type::I64]);
    b.brif(e, c, t, vec![], f, vec![]);
    let tv = b.iconst(t, Type::I64, 1);
    b.jump(t, j, vec![tv]);
    let fv = b.iconst(f, Type::I64, 2);
    b.jump(f, j, vec![fv]);
    let p = b.func().block(j).params[0];
    b.ret(j, vec![p]);
    b.into_func()
}

#[test]
fn converts_simple_diamond() {
    let mut func = build_constant_diamond();
    assert_eq!(run(&func, &[Val::B(true)]).unwrap(), vec![Val::I(1)]);
    assert!(if_convert::run(&mut func));
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(run(&func, &[Val::B(true)]).unwrap(), vec![Val::I(1)]);
    assert_eq!(run(&func, &[Val::B(false)]).unwrap(), vec![Val::I(2)]);
    assert!(!func.blocks[0]
        .insts
        .iter()
        .any(|i| matches!(i, Inst::Brif(..))));
}

#[test]
fn converts_arm_using_head_value() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::Bool]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let c = b.func().block(e).params[1];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![Type::I64]);
    b.brif(e, c, t, vec![], f, vec![]);
    let one = b.iconst(t, Type::I64, 1);
    let tv = b.bin(t, BinOp::Add, Type::I64, x, one);
    b.jump(t, j, vec![tv]);
    let two = b.iconst(f, Type::I64, 2);
    let fv = b.bin(f, BinOp::Add, Type::I64, x, two);
    b.jump(f, j, vec![fv]);
    let p = b.func().block(j).params[0];
    b.ret(j, vec![p]);

    let mut func = b.into_func();
    assert_eq!(
        run(&func, &[Val::I(10), Val::B(true)]).unwrap(),
        vec![Val::I(11)]
    );
    assert!(if_convert::run(&mut func));
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(
        run(&func, &[Val::I(10), Val::B(true)]).unwrap(),
        vec![Val::I(11)]
    );
    assert_eq!(
        run(&func, &[Val::I(10), Val::B(false)]).unwrap(),
        vec![Val::I(12)]
    );
    optimize(&mut func);
    assert_eq!(
        run(&func, &[Val::I(5), Val::B(false)]).unwrap(),
        vec![Val::I(7)]
    );
}

#[test]
fn skips_arm_with_division() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64, Type::Bool]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let c = b.func().block(e).params[2];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![Type::I64]);
    b.brif(e, c, t, vec![], f, vec![]);
    let tv = b.bin(t, BinOp::Udiv, Type::I64, x, y);
    b.jump(t, j, vec![tv]);
    let fv = b.iconst(f, Type::I64, 0);
    b.jump(f, j, vec![fv]);
    let p = b.func().block(j).params[0];
    b.ret(j, vec![p]);

    let mut func = b.into_func();
    assert!(!if_convert::run(&mut func));
    assert_eq!(
        run(&func, &[Val::I(8), Val::I(2), Val::B(true)]).unwrap(),
        vec![Val::I(4)]
    );
}

#[test]
fn skips_arm_with_params() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::Bool]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let c = b.func().block(e).params[1];
    let t = b.add_block(vec![Type::I64]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![Type::I64]);
    b.brif(e, c, t, vec![x], f, vec![]);
    let tp = b.func().block(t).params[0];
    b.jump(t, j, vec![tp]);
    let fv = b.iconst(f, Type::I64, 0);
    b.jump(f, j, vec![fv]);
    let p = b.func().block(j).params[0];
    b.ret(j, vec![p]);

    let mut func = b.into_func();
    assert!(!if_convert::run(&mut func));
    assert_eq!(
        run(&func, &[Val::I(7), Val::B(true)]).unwrap(),
        vec![Val::I(7)]
    );
}

#[test]
fn skips_large_arm() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::Bool]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let c = b.func().block(e).params[1];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![Type::I64]);
    b.brif(e, c, t, vec![], f, vec![]);
    let one = b.iconst(t, Type::I64, 1);
    let mut v = x;
    for _ in 0..9 {
        v = b.bin(t, BinOp::Add, Type::I64, v, one);
    }
    b.jump(t, j, vec![v]);
    let fv = b.iconst(f, Type::I64, 0);
    b.jump(f, j, vec![fv]);
    let p = b.func().block(j).params[0];
    b.ret(j, vec![p]);

    let mut func = b.into_func();
    assert!(!if_convert::run(&mut func));
}

#[test]
fn converts_return_arms() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::Bool]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let c = b.func().block(e).params[1];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    b.brif(e, c, t, vec![], f, vec![]);
    let one = b.iconst(t, Type::I64, 1);
    let tv = b.bin(t, BinOp::Add, Type::I64, x, one);
    b.ret(t, vec![tv]);
    let two = b.iconst(f, Type::I64, 2);
    let fv = b.bin(f, BinOp::Add, Type::I64, x, two);
    b.ret(f, vec![fv]);

    let mut func = b.into_func();
    assert_eq!(
        run(&func, &[Val::I(10), Val::B(true)]).unwrap(),
        vec![Val::I(11)]
    );
    assert!(if_convert::run(&mut func));
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(
        run(&func, &[Val::I(10), Val::B(true)]).unwrap(),
        vec![Val::I(11)]
    );
    assert_eq!(
        run(&func, &[Val::I(10), Val::B(false)]).unwrap(),
        vec![Val::I(12)]
    );
    assert_eq!(func.blocks.len(), 1);
}

#[test]
fn converts_two_value_join() {
    let mut b = Builder::new(
        vec![Type::I64],
        vec![Type::Bool, Type::I64, Type::I64],
    );
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let x = b.func().block(e).params[1];
    let y = b.func().block(e).params[2];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![Type::I64, Type::I64]);
    b.brif(e, c, t, vec![], f, vec![]);
    b.jump(t, j, vec![x, x]);
    b.jump(f, j, vec![y, y]);
    let p0 = b.func().block(j).params[0];
    let p1 = b.func().block(j).params[1];
    let sum = b.bin(j, BinOp::Add, Type::I64, p0, p1);
    b.ret(j, vec![sum]);

    let mut func = b.into_func();
    assert_eq!(
        run(&func, &[Val::B(true), Val::I(4), Val::I(9)]).unwrap(),
        vec![Val::I(8)]
    );
    assert!(if_convert::run(&mut func));
    assert!(verify(&func).is_ok(), "{:?}", verify(&func).err());
    assert_eq!(
        run(&func, &[Val::B(true), Val::I(4), Val::I(9)]).unwrap(),
        vec![Val::I(8)]
    );
    assert_eq!(
        run(&func, &[Val::B(false), Val::I(4), Val::I(9)]).unwrap(),
        vec![Val::I(18)]
    );
    assert_eq!(
        func.blocks[0]
            .insts
            .iter()
            .filter(|i| matches!(i, Inst::Select(..)))
            .count(),
        2
    );
}
