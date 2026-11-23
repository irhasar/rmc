use rakit_ir::ir::{External, Inst, Type};
use rakit_ir::{Builder, optimize, run, verify};

fn with_external(mut f: rakit_ir::Function) -> rakit_ir::Function {
    f.externals.push(External {
        namespace: 0,
        index: 1,
        params: vec![Type::I64],
        ret: None,
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
        namespace: 0,
        index: 1,
        params: vec![Type::I64],
        ret: Some(Type::I64),
    });

    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    optimize(&mut f);
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Call(..))));
}
