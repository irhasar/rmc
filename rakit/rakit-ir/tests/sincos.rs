use rakit_ir::ir::{External, ExternalTarget, Inst, Type};
use rakit_ir::{Builder, Val, run_with, verify};

#[test]
fn fuses_sinf_cosf() {
    let mut b = Builder::new(vec![Type::F32, Type::F32], vec![Type::F32]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let s = b.call(e, 0, vec![x], Some(Type::F32)).unwrap();
    let c = b.call(e, 1, vec![x], Some(Type::F32)).unwrap();
    b.ret(e, vec![s, c]);
    let mut f = b.into_func();
    f.externals.push(External {
        target: ExternalTarget::LibCall("sinf".to_string()),
        params: vec![Type::F32],
        rets: vec![Type::F32],
    });
    f.externals.push(External {
        target: ExternalTarget::LibCall("cosf".to_string()),
        params: vec![Type::F32],
        rets: vec![Type::F32],
    });
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert!(rakit_ir::sincos::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Call(r, _, _) if r.len() == 2)));
    let out = run_with(&f, &[Val::F32(0.5)], &|target, _args| {
        assert_eq!(f.externals[target as usize].target, ExternalTarget::LibCall("sincosf".into()));
        Ok(vec![Val::F32(1.0), Val::F32(2.0)])
    })
    .unwrap();
    assert_eq!(out, vec![Val::F32(1.0), Val::F32(2.0)]);
}

#[test]
fn leaves_unrelated_calls_alone() {
    let mut b = Builder::new(vec![Type::F64], vec![Type::F64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let s = b.call(e, 0, vec![x], Some(Type::F64)).unwrap();
    b.ret(e, vec![s]);
    let mut f = b.into_func();
    f.externals.push(External {
        target: ExternalTarget::LibCall("sin".to_string()),
        params: vec![Type::F64],
        rets: vec![Type::F64],
    });
    assert!(!rakit_ir::sincos::run(&mut f));
}
