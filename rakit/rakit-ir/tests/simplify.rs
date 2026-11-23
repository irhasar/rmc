use rakit_ir::ir::{BinOp, Inst, Type};
use rakit_ir::{run, simplify, verify, Builder, Val};

#[test]
fn removes_unreachable_block() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let dead = b.add_block(vec![]);
    let y = b.iconst(dead, Type::I64, 99);
    b.ret(dead, vec![y]);
    b.ret(e, vec![x]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[Val::I(7)]).unwrap(), vec![Val::I(7)]);
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(7)]).unwrap(), vec![Val::I(7)]);
}

#[test]
fn collapses_forwarding_block() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let fwd = b.add_block(vec![]);
    let target = b.add_block(vec![]);
    b.jump(e, fwd, vec![]);
    b.jump(fwd, target, vec![]);
    b.ret(target, vec![x]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[Val::I(5)]).unwrap(), vec![Val::I(5)]);
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(5)]).unwrap(), vec![Val::I(5)]);
}

#[test]
fn collapses_forwarding_chain() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let f1 = b.add_block(vec![]);
    let f2 = b.add_block(vec![]);
    let target = b.add_block(vec![]);
    b.jump(e, f1, vec![]);
    b.jump(f1, f2, vec![]);
    b.jump(f2, target, vec![]);
    b.ret(target, vec![x]);

    let mut f = b.into_func();
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(5)]).unwrap(), vec![Val::I(5)]);
}

#[test]
fn merges_blocks_keeping_used_nop() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let fwd = b.add_block(vec![]);
    let target = b.add_block(vec![]);
    b.jump(e, fwd, vec![]);
    let v = b.nop(fwd, Type::I64);
    b.jump(fwd, target, vec![]);
    b.ret(target, vec![v]);

    let mut f = b.into_func();
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert!(f.blocks[0].insts.iter().any(|i| matches!(i, Inst::Nop(..))));
}

#[test]
fn redirects_brif_edges() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::Bool]);
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![]);
    b.brif(e, c, t, vec![], f, vec![]);
    b.jump(t, j, vec![]);
    let one = b.iconst(j, Type::I64, 1);
    b.ret(j, vec![one]);
    let two = b.iconst(f, Type::I64, 2);
    b.ret(f, vec![two]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[Val::B(true)]).unwrap(), vec![Val::I(1)]);
    assert_eq!(run(&f, &[Val::B(false)]).unwrap(), vec![Val::I(2)]);
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 3);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::B(true)]).unwrap(), vec![Val::I(1)]);
    assert_eq!(run(&f, &[Val::B(false)]).unwrap(), vec![Val::I(2)]);
}

#[test]
fn removes_unreachable_chain() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let d1 = b.add_block(vec![]);
    let d2 = b.add_block(vec![]);
    b.jump(d1, d2, vec![]);
    let y = b.iconst(d2, Type::I64, 3);
    b.ret(d2, vec![y]);
    b.ret(e, vec![x]);

    let mut f = b.into_func();
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(4)]).unwrap(), vec![Val::I(4)]);
}

#[test]
fn merges_entry_target() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let target = b.add_block(vec![]);
    b.jump(e, target, vec![]);
    let one = b.iconst(target, Type::I64, 1);
    b.ret(target, vec![one]);

    let mut f = b.into_func();
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(1)]);
}

#[test]
fn is_idempotent() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let fwd = b.add_block(vec![]);
    let target = b.add_block(vec![]);
    b.jump(e, fwd, vec![]);
    b.jump(fwd, target, vec![]);
    b.ret(target, vec![x]);

    let mut f = b.into_func();
    assert!(simplify::run(&mut f));
    assert!(!simplify::run(&mut f));
}

#[test]
fn merges_single_pred_chain_into_entry() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let b1 = b.add_block(vec![]);
    let b2 = b.add_block(vec![]);
    b.jump(e, b1, vec![]);
    let one = b.iconst(b1, Type::I64, 1);
    let a = b.bin(b1, BinOp::Add, Type::I64, x, one);
    b.jump(b1, b2, vec![]);
    let one2 = b.iconst(b2, Type::I64, 1);
    let c = b.bin(b2, BinOp::Add, Type::I64, a, one2);
    b.ret(b2, vec![c]);

    let mut f = b.into_func();
    assert_eq!(run(&f, &[Val::I(2)]).unwrap(), vec![Val::I(4)]);
    assert!(simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 1);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(run(&f, &[Val::I(2)]).unwrap(), vec![Val::I(4)]);
}

#[test]
fn does_not_merge_brif_target() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::Bool]);
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    b.brif(e, c, t, vec![], f, vec![]);
    let one = b.iconst(t, Type::I64, 1);
    b.ret(t, vec![one]);
    let two = b.iconst(f, Type::I64, 2);
    b.ret(f, vec![two]);

    let mut f = b.into_func();
    assert!(!simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 3);
}

#[test]
fn does_not_merge_two_preds() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::Bool]);
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let t = b.add_block(vec![]);
    let f = b.add_block(vec![]);
    let j = b.add_block(vec![]);
    b.brif(e, c, t, vec![], f, vec![]);
    let _one = b.iconst(t, Type::I64, 1);
    b.jump(t, j, vec![]);
    let _two = b.iconst(f, Type::I64, 2);
    b.jump(f, j, vec![]);
    let three = b.iconst(j, Type::I64, 3);
    b.ret(j, vec![three]);

    let mut f = b.into_func();
    assert!(!simplify::run(&mut f));
    assert_eq!(f.blocks.len(), 4);
}

#[test]
fn redirects_try_call_continuation() {
    use rakit_ir::ir::{ExceptionItem, External, ExternalTarget, TryBlockCall};

    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let normal = b.add_block(vec![]);
    let target = b.add_block(vec![]);
    b.try_call(
        e,
        0,
        vec![],
        vec![],
        TryBlockCall { block: normal, args: vec![] },
        Vec::<ExceptionItem>::new(),
    );
    b.jump(normal, target, vec![]);
    let one = b.iconst(target, Type::I64, 1);
    b.ret(target, vec![one]);
    let mut f = b.into_func();
    f.externals.push(External {
        target: ExternalTarget::User { namespace: 0, index: 4 },
        params: vec![],
        rets: vec![],
    });

    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert!(simplify::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(f.blocks.len(), 2);
    match f.blocks[0].terminator().unwrap() {
        Inst::TryCall { normal, .. } => assert_eq!(normal.block.0 as usize, 1),
        other => panic!("expected try-call, got {other:?}"),
    }
}
