use rakit_ir::ir::{BinOp, Type, UnOp};
use rakit_ir::{Builder, Val, run};

fn unary(ret: Type, arg: Type, op: UnOp, out: Type, value: Val) -> Result<Vec<Val>, String> {
    let mut b = Builder::new(vec![ret], vec![arg]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let r = b.unary(e, op, out, x);
    b.ret(e, vec![r]);
    run(&b.into_func(), &[value])
}

#[test]
fn sextend_narrow_to_i128_preserves_sign() {
    assert_eq!(
        unary(Type::I128, Type::I8, UnOp::Sextend, Type::I128, Val::I(0xff)).unwrap(),
        vec![Val::I128(u128::MAX)]
    );
}

#[test]
fn fcvt_from_int_sign_extends_narrow_source() {
    assert_eq!(
        unary(Type::F64, Type::I8, UnOp::FcvtFromInt, Type::F64, Val::I(0xff)).unwrap(),
        vec![Val::F64(-1.0)]
    );
    assert_eq!(
        unary(Type::F64, Type::I128, UnOp::FcvtFromInt, Type::F64, Val::I128(u128::MAX))
            .unwrap(),
        vec![Val::F64(-1.0)]
    );
}

#[test]
fn fcvt_from_uint_keeps_full_i128_payload() {
    assert_eq!(
        unary(Type::F64, Type::I128, UnOp::FcvtFromUint, Type::F64, Val::I128(1u128 << 100))
            .unwrap(),
        vec![Val::F64((1u128 << 100) as f64)]
    );
}

#[test]
fn fcvt_to_int_traps_on_overflow_and_nan() {
    assert!(unary(Type::I8, Type::F64, UnOp::FcvtToInt, Type::I8, Val::F64(128.0)).is_err());
    assert!(unary(Type::I64, Type::F64, UnOp::FcvtToUint, Type::I64, Val::F64(f64::NAN)).is_err());
    assert_eq!(
        unary(Type::I8, Type::F64, UnOp::FcvtToInt, Type::I8, Val::F64(127.9)).unwrap(),
        vec![Val::I(127)]
    );
    assert_eq!(
        unary(Type::I64, Type::F64, UnOp::FcvtToUint, Type::I64, Val::F64(255.9)).unwrap(),
        vec![Val::I(255)]
    );
}

#[test]
fn pointer_signed_ops_use_integer_semantics() {
    let mut b = Builder::new(vec![Type::Ptr], vec![Type::Ptr, Type::Ptr]);
    let e = b.func().entry;
    let x = b.func().block(e).params[0];
    let y = b.func().block(e).params[1];
    let r = b.bin(e, BinOp::Shr, Type::Ptr, x, y);
    b.ret(e, vec![r]);
    assert_eq!(
        run(&b.into_func(), &[Val::P(0x8000_0000_0000_0000), Val::P(1)]).unwrap(),
        vec![Val::P(0xc000_0000_0000_0000)]
    );
}

#[test]
fn pointer_iconst_stays_a_pointer() {
    let mut b = Builder::new(vec![Type::Ptr], vec![]);
    let e = b.func().entry;
    let p = b.iconst(e, Type::Ptr, -1);
    b.ret(e, vec![p]);
    assert_eq!(run(&b.into_func(), &[]).unwrap(), vec![Val::P(u64::MAX)]);
}
