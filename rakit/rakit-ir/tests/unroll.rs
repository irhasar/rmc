use rakit_ir::ir::{BinOp, IntCc, Type};
use rakit_ir::{Builder, Val, run, unroll, verify};

#[test]
fn unrolls_counted_loop() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let zero = b.iconst(e, Type::I64, 0);
    let header = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![Type::I64]);
    b.jump(e, header, vec![zero, zero]);

    let iv = b.func().block(header).params[0];
    let acc = b.func().block(header).params[1];
    let bound = b.iconst(header, Type::I64, 4);
    let cond = b.icmp(header, IntCc::Ult, Type::I64, iv, bound);
    b.brif(header, cond, body, vec![], exit, vec![acc]);

    let one = b.iconst(body, Type::I64, 1);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    let acc2 = b.bin(body, BinOp::Add, Type::I64, acc, iv);
    b.jump(body, header, vec![iv2, acc2]);

    let out = b.func().block(exit).params[0];
    b.ret(exit, vec![out]);

    let mut f = b.into_func();
    assert!(unroll::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(6)]);
}
