use rakit_ir::ir::{External, ExternalTarget, Inst, Type};
use rakit_ir::{Builder, Val, inliner, run, verify};

#[test]
fn inlines_single_block_leaf() {
    let target = ExternalTarget::User { namespace: 0, index: 7 };

    let mut callee = Builder::new(vec![Type::I64], vec![Type::I64]);
    let ce = callee.func().entry;
    let x = callee.func().block(ce).params[0];
    let one = callee.iconst(ce, Type::I64, 1);
    let y = callee.bin(ce, rakit_ir::ir::BinOp::Add, Type::I64, x, one);
    callee.ret(ce, vec![y]);
    let mut callee = callee.into_func();
    callee.self_target = Some(target.clone());

    let mut caller = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = caller.func().entry;
    let arg = caller.func().block(e).params[0];
    let r = caller.call_multi(e, 0, vec![arg], vec![Type::I64])[0];
    caller.ret(e, vec![r]);
    let mut caller = caller.into_func();
    caller.externals.push(External {
        target,
        params: vec![Type::I64],
        rets: vec![Type::I64],
    });
    assert!(inliner::run(&mut caller, &[callee]));
    assert!(verify(&caller).is_ok(), "{:?}", verify(&caller));
    assert!(!caller.blocks[0]
        .insts
        .iter()
        .any(|i| matches!(i, Inst::Call(..))));
    assert_eq!(run(&caller, &[Val::I(41)]).unwrap(), vec![Val::I(42)]);
}

#[test]
fn inlines_leaf_whose_value_ids_exceed_caller_table() {
    let target = ExternalTarget::User { namespace: 0, index: 8 };

    // Callee has three parameters and two computed results.  Its result ids
    // (3 and 4) are outside the caller's small value table; cloning must use
    // the callee's type table.
    let mut callee = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64, Type::I64]);
    let ce = callee.func().entry;
    let p = callee.func().block(ce).params.clone();
    let s = callee.bin(ce, rakit_ir::ir::BinOp::Add, Type::I64, p[0], p[1]);
    let t = callee.bin(ce, rakit_ir::ir::BinOp::Add, Type::I64, s, p[2]);
    callee.ret(ce, vec![t]);
    let mut callee = callee.into_func();
    callee.self_target = Some(target.clone());

    let mut caller = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = caller.func().entry;
    let arg = caller.func().block(e).params[0];
    let r = caller.call_multi(e, 0, vec![arg, arg, arg], vec![Type::I64])[0];
    caller.ret(e, vec![r]);
    let mut caller = caller.into_func();
    caller.externals.push(External {
        target,
        params: vec![Type::I64, Type::I64, Type::I64],
        rets: vec![Type::I64],
    });

    assert!(inliner::run(&mut caller, &[callee]));
    assert!(verify(&caller).is_ok(), "{:?}", verify(&caller));
    assert_eq!(run(&caller, &[Val::I(10)]).unwrap(), vec![Val::I(30)]);
}
