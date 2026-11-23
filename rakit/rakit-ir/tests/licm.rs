use rakit_ir::ir::{BinOp, Inst, IntCc, Type};
use rakit_ir::{licm, run, verify, Builder, Val};

#[test]
fn hoists_loop_invariant() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let n = b.func().block(e).params[2];
    let h = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![]);
    let z0 = b.iconst(e, Type::I64, 0);
    let z1 = b.iconst(e, Type::I64, 0);
    b.jump(e, h, vec![z0, z1]);
    let iv = b.func().block(h).params[0];
    let s = b.func().block(h).params[1];
    let c = b.icmp(h, IntCc::Ult, Type::I64, iv, n);
    b.brif(h, c, body, vec![], exit, vec![]);
    let t = b.bin(body, BinOp::Mul, Type::I64, x, y);
    let s2 = b.bin(body, BinOp::Add, Type::I64, s, t);
    let one = b.iconst(body, Type::I64, 1);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    b.jump(body, h, vec![iv2, s2]);
    b.ret(exit, vec![s]);

    let mut f = b.into_func();
    assert_eq!(
        run(&f, &[Val::I(2), Val::I(3), Val::I(4)]).unwrap(),
        vec![Val::I(24)]
    );
    assert!(licm::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(
        run(&f, &[Val::I(2), Val::I(3), Val::I(4)]).unwrap(),
        vec![Val::I(24)]
    );
    assert!(f.blocks[0]
        .insts
        .iter()
        .any(|i| matches!(i, Inst::Bin(_, BinOp::Mul, ..))));
    assert!(!licm::run(&mut f));
}

#[test]
fn skips_loop_dependent() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64]);
    let e = b.func().entry;
    let n = b.func().block(e).params[0];
    let h = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![]);
    let z0 = b.iconst(e, Type::I64, 0);
    let z1 = b.iconst(e, Type::I64, 0);
    b.jump(e, h, vec![z0, z1]);
    let iv = b.func().block(h).params[0];
    let s = b.func().block(h).params[1];
    let c = b.icmp(h, IntCc::Ult, Type::I64, iv, n);
    b.brif(h, c, body, vec![], exit, vec![]);
    let s2 = b.bin(body, BinOp::Add, Type::I64, s, s);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, iv);
    b.jump(body, h, vec![iv2, s2]);
    b.ret(exit, vec![s]);

    let mut f = b.into_func();
    assert!(!licm::run(&mut f));
}

#[test]
fn skips_loop_without_preheader() {
    let mut b = Builder::new(
        vec![Type::I64],
        vec![Type::Bool, Type::I64, Type::I64, Type::I64],
    );
    let e = b.func().entry;
    let c = b.func().block(e).params[0];
    let x = b.func().block(e).params[1];
    let y = b.func().block(e).params[2];
    let n = b.func().block(e).params[3];
    let a = b.add_block(vec![]);
    let d = b.add_block(vec![]);
    let h = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![]);
    b.brif(e, c, a, vec![], d, vec![]);
    let za = b.iconst(a, Type::I64, 0);
    let zb = b.iconst(a, Type::I64, 0);
    b.jump(a, h, vec![za, zb]);
    let zc = b.iconst(d, Type::I64, 0);
    let zd = b.iconst(d, Type::I64, 0);
    b.jump(d, h, vec![zc, zd]);
    let iv = b.func().block(h).params[0];
    let s = b.func().block(h).params[1];
    let c2 = b.icmp(h, IntCc::Ult, Type::I64, iv, n);
    b.brif(h, c2, body, vec![], exit, vec![]);
    let t = b.bin(body, BinOp::Mul, Type::I64, x, y);
    let s2 = b.bin(body, BinOp::Add, Type::I64, s, t);
    let one = b.iconst(body, Type::I64, 1);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    b.jump(body, h, vec![iv2, s2]);
    b.ret(exit, vec![s]);

    let mut f = b.into_func();
    assert!(!licm::run(&mut f));
}

#[test]
fn hoists_chain_in_rounds() {
    let mut b = Builder::new(vec![Type::I64], vec![Type::I64, Type::I64, Type::I64]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let n = b.func().block(e).params[2];
    let h = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![]);
    let z0 = b.iconst(e, Type::I64, 0);
    let z1 = b.iconst(e, Type::I64, 0);
    b.jump(e, h, vec![z0, z1]);
    let iv = b.func().block(h).params[0];
    let s = b.func().block(h).params[1];
    let c = b.icmp(h, IntCc::Ult, Type::I64, iv, n);
    b.brif(h, c, body, vec![], exit, vec![]);
    let t1 = b.bin(body, BinOp::Mul, Type::I64, x, y);
    let one = b.iconst(body, Type::I64, 1);
    let t2 = b.bin(body, BinOp::Add, Type::I64, t1, one);
    let s2 = b.bin(body, BinOp::Add, Type::I64, s, t2);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    b.jump(body, h, vec![iv2, s2]);
    b.ret(exit, vec![s]);

    let mut f = b.into_func();
    assert_eq!(
        run(&f, &[Val::I(2), Val::I(3), Val::I(4)]).unwrap(),
        vec![Val::I(28)]
    );
    assert!(licm::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f).err());
    assert_eq!(
        run(&f, &[Val::I(2), Val::I(3), Val::I(4)]).unwrap(),
        vec![Val::I(28)]
    );
    assert!(!licm::run(&mut f));
}
