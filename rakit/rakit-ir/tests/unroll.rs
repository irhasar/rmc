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

#[test]
fn header_value_used_by_body_is_not_reused_stale() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let zero = b.iconst(e, Type::I64, 0);
    let header = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![Type::I64]);
    b.jump(e, header, vec![zero, zero]);

    let iv = b.func().block(header).params[0];
    let acc = b.func().block(header).params[1];
    let two = b.iconst(header, Type::I64, 2);
    let term = b.bin(header, BinOp::Mul, Type::I64, iv, two);
    let bound = b.iconst(header, Type::I64, 4);
    let cond = b.icmp(header, IntCc::Ult, Type::I64, iv, bound);
    b.brif(header, cond, body, vec![], exit, vec![acc]);

    let one = b.iconst(body, Type::I64, 1);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    let acc2 = b.bin(body, BinOp::Add, Type::I64, acc, term);
    b.jump(body, header, vec![iv2, acc2]);

    let out = b.func().block(exit).params[0];
    b.ret(exit, vec![out]);

    let mut f = b.into_func();
    let changed = unroll::run(&mut f);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(12)], "changed={changed}");
}

#[test]
fn conditional_second_entry_does_not_overshoot() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let zero = b.iconst(e, Type::I64, 0);
    let three = b.iconst(e, Type::I64, 3);
    let yes = b.iconst(e, Type::Bool, 1);
    let pre = b.add_block(vec![]);
    let alt = b.add_block(vec![]);
    let header = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![Type::I64]);
    b.brif(e, yes, alt, vec![], pre, vec![]);
    b.jump(pre, header, vec![zero, zero]);

    let yes2 = b.iconst(alt, Type::Bool, 1);
    b.brif(alt, yes2, header, vec![three, zero], exit, vec![zero]);

    let iv = b.func().block(header).params[0];
    let acc = b.func().block(header).params[1];
    let bound = b.iconst(header, Type::I64, 4);
    let cond = b.icmp(header, IntCc::Ult, Type::I64, iv, bound);
    b.brif(header, cond, body, vec![], exit, vec![acc]);

    let one = b.iconst(body, Type::I64, 1);
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    let acc2 = b.bin(body, BinOp::Add, Type::I64, acc, one);
    b.jump(body, header, vec![iv2, acc2]);

    let out = b.func().block(exit).params[0];
    b.ret(exit, vec![out]);

    let mut f = b.into_func();
    let changed = unroll::run(&mut f);
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(1)], "changed={changed}");
}

#[test]
fn dead_nop_in_header_does_not_block_unrolling() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let zero = b.iconst(e, Type::I64, 0);
    let header = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![Type::I64]);
    b.jump(e, header, vec![zero, zero]);

    // Dead-code elimination leaves Nop placeholders behind. They have no
    // effect and must not make an otherwise pure header look unsafe.
    b.nop(header, Type::I64);

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

#[test]
fn back_edge_header_update_is_not_reused_stale() {
    let mut b = Builder::new(vec![Type::I64], vec![]);
    let e = b.func().entry;
    let zero = b.iconst(e, Type::I64, 0);
    let header = b.add_block(vec![Type::I64, Type::I64]);
    let body = b.add_block(vec![]);
    let exit = b.add_block(vec![Type::I64]);
    b.jump(e, header, vec![zero, zero]);

    let iv = b.func().block(header).params[0];
    let acc = b.func().block(header).params[1];
    let one = b.iconst(header, Type::I64, 1);
    let next_acc = b.bin(header, BinOp::Add, Type::I64, acc, one);
    let bound = b.iconst(header, Type::I64, 4);
    let cond = b.icmp(header, IntCc::Ult, Type::I64, iv, bound);
    b.brif(header, cond, body, vec![], exit, vec![acc]);

    // The body only advances iv; the header-computed `acc + 1` is the loop
    // update. Cloned bodies must not reuse a single header result.
    let iv2 = b.bin(body, BinOp::Add, Type::I64, iv, one);
    b.jump(body, header, vec![iv2, next_acc]);

    let out = b.func().block(exit).params[0];
    b.ret(exit, vec![out]);

    let mut f = b.into_func();
    assert!(!unroll::run(&mut f), "unroll must reject a changing header update");
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert_eq!(run(&f, &[]).unwrap(), vec![Val::I(4)]);
}
