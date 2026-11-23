use rakit_ir::ir::{External, ExternalTarget, Inst, Type};
use rakit_ir::{Builder, Val, optimize, run, run_with, run_with_indirect, verify};

fn with_external(mut f: rakit_ir::Function) -> rakit_ir::Function {
    f.externals.push(External {
        target: ExternalTarget::User { namespace: 0, index: 1 },
        params: vec![Type::I64],
        rets: vec![],
    });
    f
}

#[test]
fn verifies_and_keeps_call_without_result() {
    let mut b = Builder::new(vec![], vec![]);
    let e = b.func().entry;
    let x = b.iconst(e, Type::I64, 1);
    b.call(e, 0, vec![x], None);
    b.ret(e, vec![]);
    let mut f = with_external(b.into_func());

    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    optimize(&mut f);
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Call(..))));
    assert!(run(&f, &[]).is_err());
}

#[test]
fn verifies_and_keeps_call_with_result() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let r = b.call(e, 0, vec![x], Some(Type::I64)).unwrap();
    b.ret(e, vec![r]);
    let mut f = b.into_func();
    f.externals.push(External {
        target: ExternalTarget::User { namespace: 0, index: 1 },
        params: vec![Type::I64],
        rets: vec![Type::I64],
    });

    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    optimize(&mut f);
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Call(..))));
}

#[test]
fn interpreter_runs_external_call() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let r = b.call(e, 0, vec![x], Some(Type::I64)).unwrap();
    b.ret(e, vec![r]);
    let mut f = b.into_func();
    f.externals.push(External {
        target: ExternalTarget::User { namespace: 0, index: 1 },
        params: vec![Type::I64],
        rets: vec![Type::I64],
    });

    let out = run_with(&f, &[Val::I(41)], &|_, args| match args[0] {
        Val::I(v) => Ok(vec![Val::I(v + 1)]),
        _ => Err("expected integer".to_string()),
    })
    .unwrap();
    assert_eq!(out, vec![Val::I(42)]);
}

#[test]
fn interpreter_runs_multi_result_external_call() {
    let mut b = Builder::new(vec![Type::I64, Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let results = b.call_multi(e, 0, vec![x], vec![Type::I64, Type::I64]);
    b.ret(e, results);
    let mut f = b.into_func();
    f.externals.push(External {
        target: ExternalTarget::User { namespace: 0, index: 1 },
        params: vec![Type::I64],
        rets: vec![Type::I64, Type::I64],
    });

    let out = run_with(&f, &[Val::I(3)], &|_, args| match args[0] {
        Val::I(v) => Ok(vec![Val::I(v), Val::I(v * 2)]),
        _ => Err("expected integer".to_string()),
    })
    .unwrap();
    assert_eq!(out, vec![Val::I(3), Val::I(6)]);
}

#[test]
fn interpreter_runs_indirect_call() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64]);
    let e = b.func().entry;
    let callee = b.func().block(e).params[0];
    let x = b.func().block(e).params[1];
    let results = b.call_indirect(e, vec![Type::I64], vec![Type::I64], 2, callee, vec![x]);
    b.ret(e, results);
    let f = b.into_func();
    assert!(verify(&f).is_ok());

    let out = run_with_indirect(
        &f,
        &[Val::I(7), Val::I(41)],
        &|_, _| Err("direct call not expected".to_string()),
        &|callee, args| match (callee, args[0]) {
            (Val::I(7), Val::I(41)) => Ok(vec![Val::I(42)]),
            _ => Err("unexpected indirect target".to_string()),
        },
    )
    .unwrap();
    assert_eq!(out, vec![Val::I(42)]);
}
