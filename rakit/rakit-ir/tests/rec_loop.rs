use rakit_ir::ir::{
    BinOp, Block, BlockData, External, ExternalTarget, Function, Inst, IntCc, Type, Value,
};
use rakit_ir::{rec_loop, verify};

#[test]
fn rewrites_recurrence_to_one_self_call_per_iteration() {
    let target = ExternalTarget::User { namespace: 0, index: 0 };
    let mut f = Function::new(vec![Type::I64], vec![Type::I64]);
    f.self_target = Some(target.clone());
    f.externals.push(External {
        target,
        params: vec![Type::I64],
        rets: vec![Type::I64],
    });

    let n = f.blocks[0].params[0];
    f.blocks.push(BlockData::default());
    f.blocks.push(BlockData::default());

    let mut v = |ty| {
        let value = Value(f.values.len() as u32);
        f.values.push(ty);
        value
    };
    let base = v(Type::I64);
    let cond = v(Type::Bool);
    let c1 = v(Type::I64);
    let n1 = v(Type::I64);
    let t1 = v(Type::I64);
    let c2 = v(Type::I64);
    let n2 = v(Type::I64);
    let t2 = v(Type::I64);
    let sum = v(Type::I64);

    f.blocks[0].insts = vec![
        Inst::Iconst(base, Type::I64, 2),
        Inst::Icmp(cond, IntCc::Ult, Type::I64, n, base),
        Inst::Brif(cond, Block(1), vec![], Block(2), vec![]),
    ];
    f.blocks[1].insts = vec![Inst::Return(vec![n])];
    f.blocks[2].insts = vec![
        Inst::Iconst(c1, Type::I64, 1),
        Inst::Bin(n1, BinOp::Sub, Type::I64, n, c1),
        Inst::Call(vec![t1], 0, vec![n1]),
        Inst::Iconst(c2, Type::I64, 1),
        Inst::Bin(n2, BinOp::Sub, Type::I64, n, c2),
        Inst::Call(vec![t2], 0, vec![n2]),
        Inst::Bin(sum, BinOp::Add, Type::I64, t1, t2),
        Inst::Return(vec![sum]),
    ];

    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    assert!(rec_loop::run(&mut f));
    assert!(verify(&f).is_ok(), "{:?}", verify(&f));
    let calls = f
        .blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| matches!(i, Inst::Call(..)))
        .count();
    assert_eq!(calls, 1, "the rewritten loop must call itself once per iteration");
}
